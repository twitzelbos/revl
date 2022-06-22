/// A ring queue is composed of two lockless ring buffers and a data
/// vector: dq stores indices of messages pending receive which are
/// available at the corresponding cells from the data vector, fq
/// stores indices of free cells into the data vector. fq + dq covers
/// the whole index space, which is (1 << ORDER) long.
///
/// In practice, an index is pulled from fq, the data vector is filled
/// with a message at this index next, which is eventually pushed to
/// dq. The receiver pulls the next available index from dq, extracts
/// the message at the corresponding position in the vector then
/// releases the consumed index to fq.
///
/// The lighweight ring buffers are based on Ruslan Nikolaev's
/// Scalable Circular Queue (single-width CAS variant) ported to Rust:
/// http://drops.dagstuhl.de/opus/volltexte/2019/11335/pdf/LIPIcs-DISC-2019-28.pdf
/// https://github.com/rusnikola/lfqueue.git

use std::sync::{
    Arc,
    atomic::fence,
    atomic::AtomicUsize,
    atomic::AtomicIsize,
    atomic::Ordering::Acquire,
    atomic::Ordering::AcqRel,
    atomic::Ordering::Relaxed,
    atomic::Ordering::Release,
};
use std::mem;
use std::default::Default;
use core::cell::UnsafeCell;

// Conservative: 128 bytes should fit anything we run on. Bottom line:
// we want to prevent cacheline bouncing in SMP on hot data.
const CACHELINE_SHIFT: usize = 7;

#[cfg(target_pointer_width = "64")]
const RING_MIN_ORDER: usize = CACHELINE_SHIFT - 3;
#[cfg(target_pointer_width = "32")]
const RING_MIN_ORDER: usize = CACHELINE_SHIFT - 2;
const RING_EMPTY_VAL: usize = !0;
const RING_EMPTY_CELL: AtomicUsize = AtomicUsize::new(RING_EMPTY_VAL);

// Revisit: Rust align() attribute currently requires a literal,
// struct fields do not support alignment directives, complex const
// generics are not available from the stable channel yet, all of this
// is a bit of a pain at the moment. We just work around those
// limitations for now.

#[repr(align(128))]             // CACHELINE_ALIGNMENT
struct Head {
    d: AtomicUsize,
}

#[repr(align(128))]
struct Threshold {
    d: AtomicIsize,
}

#[repr(align(128))]
struct Tail {
    d: AtomicUsize,
}

fn sub_with_overflow(lhs: usize, rhs: usize) -> isize {
    lhs.overflowing_sub(rhs).0 as isize
}

#[repr(align(128))]
pub struct Ring<const ORDER: usize> {
    // Revisit when we have complex const generics, so that we can
    // define an array inline [ RING_EMPTY_CELL; 1 << (ORDER + 1) ].
    cells: Vec<AtomicUsize>,
    head: Head,
    threshold: Threshold,
    tail: Tail,
}

impl<const ORDER: usize> Ring<ORDER> {
    fn new() -> Self {
        let nr_cells = Ring::<ORDER>::get_nr_cells();
        let mut this = Self {
            // To maintain every single ring entry, we need two cells.
            cells: Vec::with_capacity(nr_cells),
            head: Head { d: AtomicUsize::new(0) },
            tail: Tail { d: AtomicUsize::new(0) },
            threshold: Threshold { d: AtomicIsize::new(-1) },
        };
        // Populate the vector.
        this.cells.resize_with(nr_cells, || { RING_EMPTY_CELL });
        this
    }
    fn fill(&mut self) {
        let half: usize = Ring::<ORDER>::get_nr_entries();
        let full: usize = Ring::<ORDER>::get_nr_cells();
        for n in 0..half {
            self.cells[Ring::<ORDER>::map(n, full, ORDER + 1)].store(
                Ring::<ORDER>::map(full + n, half, ORDER), Relaxed
            );
        }
        for n in half..full {
            self.cells[Ring::<ORDER>::map(n, full, ORDER + 1)].store(
                RING_EMPTY_VAL, Relaxed
            );
        }
        self.head.d.store(0, Relaxed);
        self.tail.d.store(half, Relaxed);
        self.threshold.d.store(Ring::<ORDER>::get_threshold(half, full), Relaxed);
    }
    fn enqueue(&self, eidx: usize) {
        let mut eidx = eidx;
        let half: usize = Ring::<ORDER>::get_nr_entries();
        let full: usize = Ring::<ORDER>::get_nr_cells();
        eidx ^= full - 1;
        'again: loop {
            let tail = self.tail.d.fetch_add(1, AcqRel);
            let tcycle = (tail << 1) | (2 * full - 1);
            let tidx = Ring::<ORDER>::map(tail, full, ORDER + 1);
            let mut entry = self.cells[tidx].load(Acquire);
            loop {
                let ecycle = entry | (2 * full - 1);
                if sub_with_overflow(ecycle, tcycle) < 0 &&
                    (entry == ecycle ||
                     (entry == (ecycle ^ full) &&
                      sub_with_overflow(self.head.d.load(Acquire), tail) <= 0)) {
                        match self.cells[tidx].compare_exchange_weak(entry, tcycle ^ eidx, AcqRel, Acquire) {
                            Ok(_) => break,
                            Err(ret) => entry = ret,
                        }
                    } else {
                        continue 'again;
                    }
            }
            let t = Ring::<ORDER>::get_threshold(half, full);
            if self.threshold.d.load(Relaxed) != t {
                self.threshold.d.store(t, Relaxed);
            }
            break;
        }
    }
    fn dequeue(&self) -> Option<usize> {
        if self.threshold.d.load(Relaxed) < 0 {
            return None;
        }
        let full: usize = Ring::<ORDER>::get_nr_cells();
        loop {
            let head = self.head.d.fetch_add(1, AcqRel);
            let hcycle = (head << 1) | (2 * full - 1);
            let hidx = Ring::<ORDER>::map(head, full, ORDER + 1);
            let mut attempt = 0;
            'again: loop {
                let mut entry = self.cells[hidx].load(Acquire);
                loop {
                    let ecycle = entry | (2 * full - 1);
                    if ecycle == hcycle {
                        self.cells[hidx].fetch_or(full - 1, AcqRel);
                        return Some(entry & (full - 1));
                    }
                    let new_entry;
                    if (entry | full) != ecycle {
                        new_entry = entry & (!full);
                        if entry == new_entry {
                            break;
                        }
                    } else {
                        attempt += 1;
                        if attempt <= 3000 {
                            continue 'again;
                        }
                        new_entry = hcycle ^ ((!entry) & full);
                    }
                    if sub_with_overflow(ecycle, hcycle) >= 0 {
                        break;
                    }
                    match self.cells[hidx].compare_exchange_weak(entry, new_entry, AcqRel, Acquire) {
                        Ok(_) => break,
                        Err(ret) => entry = ret,
                    }
                }
                let tail = self.tail.d.load(Acquire);
                if sub_with_overflow(tail, head + 1) <= 0 {
                    self.catchup(tail, head + 1);
                    self.threshold.d.fetch_sub(1, AcqRel);
                    return None;
                }
                if self.threshold.d.fetch_sub(1, AcqRel) <= 0 {
                    return None;
                }
            }
        }
    }
    const fn get_nr_entries() -> usize {
        1usize << ORDER
    }
    const fn get_nr_cells() -> usize {
        1usize << (ORDER + 1)
    }
    const fn map(idx: usize, limit: usize, order: usize) -> usize {
        ((idx & (limit - 1)) >> (order - RING_MIN_ORDER)) |
	 ((idx << RING_MIN_ORDER) & (limit - 1))
    }
    fn get_threshold(half: usize, nr: usize) -> isize {
	(half + nr - 1) as isize
    }
    fn catchup(&self, tail: usize, head: usize) {
        let mut head = head;
        let mut tail = tail;
        loop {
            match self.tail.d.compare_exchange_weak(tail, head, AcqRel, Acquire) {
                Ok(_) => break,
                Err(ret) => tail = ret,
            }
            head = self.head.d.load(Acquire);
            if sub_with_overflow(tail, head) >= 0 {
                break;
            }
        }
    }
}

pub struct Sender<T, const ORDER: usize> {
    rq: Arc<RingQueue<T, ORDER>>,
}

impl<T : Default, const ORDER: usize> Sender<T, ORDER> {
    pub fn send(&self, msg: T) -> Option<()> {
        self.rq.send(msg)
    }
}

impl<T: Default, const ORDER: usize> Clone for Sender<T, ORDER> {
    fn clone(&self) -> Self {
        Self { rq: self.rq.clone() }
    }
}

pub struct Receiver<T, const ORDER: usize> {
    rq: Arc<RingQueue<T, ORDER>>,
}

impl<T : Default, const ORDER: usize> Receiver<T, ORDER> {
    pub fn recv(&self) -> Option<T> {
        self.rq.recv()
    }
}

impl<T: Default, const ORDER: usize> Clone for Receiver<T, ORDER> {
    fn clone(&self) -> Self {
        Self { rq: self.rq.clone() }
    }
}

/// Memory safety on top of the UnsafeCell is guaranteed by the fact
/// that at any point in time, only a single thread can refer to any
/// given data cell, since the corresponding index in the vector is
/// never shared (no W/W conflict), and a data cell cannot be consumed
/// before it is fully populated with the message (no R/W conflict).

struct RingQueue<T, const ORDER: usize> {
    dq: Ring::<ORDER>,
    fq: Ring::<ORDER>,
    data: UnsafeCell<Vec<T>>,
}

impl<T : Default, const ORDER: usize> RingQueue<T, ORDER> {
    fn send(&self, msg: T) -> Option<()> {
        if let Some(eidx) = self.fq.dequeue() {
            fence(Release);
            unsafe { (*self.data.get())[eidx] = msg; }
            // We have as many free slots than we have data cells, so
            // enqueing cannot fail by construction.
            self.dq.enqueue(eidx);
            Some(())
        } else {
            None
        }
    }
    fn recv(&self) -> Option<T> {
        if let Some(eidx) = self.dq.dequeue() {
            // Make sure to take the message, releasing the cloned
            // references in the same move.
            let msg = unsafe { mem::take(&mut (*self.data.get())[eidx]) };
            fence(Acquire);
            self.fq.enqueue(eidx);
            Some(msg)
        } else {
            None
        }
    }
}

pub fn create<T : Default, const ORDER: usize>() -> (Sender<T, ORDER>, Receiver<T, ORDER>) {
    let nr_data = 1 << ORDER;
    let mut rq = RingQueue {
        dq: Ring::<ORDER>::new(),
        fq: Ring::<ORDER>::new(),
        data: UnsafeCell::new(Vec::with_capacity(nr_data)),
    };
    // Populate the data vector with default values, start with a full
    // free ring. Revisit: Until we have complex const generics
    // available, we need to allocate the vector separately.
    rq.data.get_mut().resize_with(nr_data, || { Default::default() });
    rq.fq.fill();
    let r = Arc::new(rq);
    ( Sender { rq: r.clone() }, Receiver { rq: r } )
}
