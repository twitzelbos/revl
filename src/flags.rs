//! Event flag group.

use std::cell::UnsafeCell;
use std::ffi::CString;
use std::io::Error;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::ptr;
use evl_sys::{
    evl_flags,
    evl_create_flags,
    evl_close_flags,
    evl_wait_flags,
    evl_trywait_flags,
    evl_peek_flags,
    evl_post_flags,
    BuiltinClock,
    CloneFlags,
};

pub struct Builder {
    name: Option<String>,
    visible: bool,
    initval: u32,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            name: None,
            visible: false,
            initval: 0u32,
        }
    }
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }
    pub fn public(mut self) -> Self {
        self.visible = true;
        self
    }
    pub fn private(mut self) -> Self {
        self.visible = false;
        self
    }
    pub fn init_value(mut self, initval: u32) -> Self {
        self.initval = initval;
        self
    }
    pub fn create(self) -> Result<Flags, Error> {
        Flags::new(self)
    }
}

pub struct Flags(UnsafeCell<evl_flags>);

unsafe impl Send for Flags {}
unsafe impl Sync for Flags {}

impl Flags {
    /// Create an EVL event flag group.
    ///
    /// # Arguments
    ///
    /// * `builder`: a builder struct containing the properties of
    /// the new flag group.
    ///
    /// # Errors
    ///
    /// * Error(AlreadyExists) means the group name is conflicting
    /// with an existing group name.
    ///
    /// * Error(InvalidInput) means that the group name contains
    /// invalid characters: such name must contain only valid
    /// characters in the context of a Linux file name.
    ///
    /// * Error(Other) may mean that either the EVL core is not
    /// enabled in the kernel, or there is an ABI mismatch between the
    /// underlying [evl-sys
    /// crate](https://source.denx.de/Xenomai/xenomai4/evl-sys) and
    /// the EVL core. See [these
    /// explanations](https://evlproject.org/core/under-the-hood/abi/)
    /// for the latter.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use revl::flags::{Builder, Flags};
    ///
    /// fn create_a_flag_group(initval: u32) -> Result<Flags, std::io::Error> {
    ///     let props = Builder::new().name("some_event_flags").public().init_value(initval);
    ///     let me = Flags::new(props)?;
    ///     Ok(me)
    /// }
    /// ```
    ///
    pub fn new(builder: Builder) -> Result<Self, Error> {
        let this = Self(UnsafeCell::new(unsafe {
            MaybeUninit::<evl_flags>::zeroed().assume_init()
        }));
        let mut c_flags = CloneFlags::PRIVATE.bits() as c_int;
        if builder.visible {
            c_flags = CloneFlags::PUBLIC.bits() as c_int;
        }
        let c_initval = builder.initval as i32;
        // Revisit: this is too restrictive.
        let c_clockfd = BuiltinClock::MONOTONIC as i32;
        let ret: c_int = unsafe {
            if let Some(name) = builder.name {
                let c_name = CString::new(name).expect("CString::new failed");
                let c_fmt = CString::new("%s").expect("CString::new failed");
                evl_create_flags(
                    this.0.get(),
                    c_clockfd,
                    c_initval,
                    c_flags,
                    c_fmt.as_ptr(),
                    c_name.as_ptr(),
                )
            } else {
                evl_create_flags(this.0.get(),
                               c_clockfd,
                               c_initval,
                               c_flags,
                               ptr::null())
            }
        };
        match ret {
            0.. => return Ok(this),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
    /// Wait for events on a flag group.
    ///
    /// Waits for events to be available from the flag group. The
    /// caller may be put to sleep by the core until this
    /// happens. Waiters are queued by order of scheduling priority.
    ///
    /// When the flag group receives some event(s), its value is
    /// passed back to the waiter leading the wait queue then reset to
    /// zero atomically before the latter returns.
    ///
    /// # Example
    ///
    /// ```rust
    /// use revl::flags::Flags;
    ///
    /// fn wait_flags(fgroup: &Flags) -> Result<u32, std::io::Error> {
    ///     fgroup.wait()
    /// }
    ///
    pub fn wait(&self) -> Result<u32, Error> {
	let mut mask = MaybeUninit::<i32>::uninit();
        let ret: c_int = unsafe { evl_wait_flags(self.0.get(), mask.as_mut_ptr()) };
        match ret {
            0 => return Ok(unsafe { mask.assume_init() } as u32),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
    /// Try receiving events from a flag group.
    ///
    /// Attempt to read from the flag group, without blocking the
    /// caller if there is none.
    ///
    /// # Example
    ///
    /// ```rustc
    /// use revl::flags::Flags;
    ///
    /// if let Some(bits) = fgroup.trywait() {
    ///    println!("ok! got events {}", bits);
    /// } else {
    ///    println!("no events pending");
    /// }
    /// ```
    ///
    pub fn trywait(&self) -> Option<u32> {
	let mut mask = MaybeUninit::<i32>::uninit();
        let ret: c_int = unsafe { evl_trywait_flags(self.0.get(), mask.as_mut_ptr()) };
        match ret {
            0 => return Some(unsafe { mask.assume_init() } as u32),
            _ => return None,
        };
    }
    /// Read the current value of a flag group.
    ///
    /// Returns the value of the flag group without blocking or
    /// altering its state (i.e. the flag group is not zeroed if some
    /// events are pending).
    ///
    /// # Example
    ///
    /// ```
    /// if let Some(bits) = fgroup.peek() {
    ///    println!("ok! got events {}", bits);
    /// } else {
    ///    println!("no events pending");
    /// }
    /// ```
    ///
    pub fn peek(&self) -> Option<u32> {
	let mut mask = MaybeUninit::<i32>::uninit();
        let ret: c_int = unsafe { evl_peek_flags(self.0.get(), mask.as_mut_ptr()) };
        match ret {
            0 => return Some(unsafe { mask.assume_init() } as u32),
            _ => return None,
        };
    }
    /// Post events to a flag group.
    ///
    /// Sends a set of events to the flag group. If a thread is
    /// sleeping on the flag group as a result of a previous call to
    /// wait(), the thread heading the wait queue is unblocked,
    /// receiving all pending events atomically.
    //
    /// # Example
    ///
    /// ```rust
    /// use revl::flags::Flags;
    ///
    /// fn post_flags(fgroup: &Flags, bits: u32) -> Result<(), std::io::Error> {
    ///     fgroup.post(bits)
    /// }
    ///
    pub fn post(&self, bits: u32) -> Result<(), Error> {
        let c_bits = bits as i32;
        let ret: c_int = unsafe { evl_post_flags(self.0.get(), c_bits) };
        match ret {
            0 => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
}

impl Drop for Flags {
    fn drop(&mut self) {
        unsafe {
            evl_close_flags(self.0.get());
        }
    }
}
