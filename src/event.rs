use std::cell::UnsafeCell;
use std::ffi::CString;
use std::io::Error;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_long};
use std::ptr;
use libc::{
    ETIMEDOUT,
    time_t,
};
use embedded_time::{
    duration::{Nanoseconds, Seconds},
    fixed_point::FixedPoint,
    Instant,
};
use evl_sys::{
    evl_event,
    evl_create_event,
    evl_close_event,
    evl_wait_event,
    evl_timedwait_event,
    evl_signal_event,
    evl_broadcast_event,
    evl_signal_thread,
    BuiltinClock,
    CloneFlags,
    timespec,
};
use crate::mutex::MutexGuard;
use crate::clock::CoreClock;
use crate::thread::Thread;

pub struct Builder {
    name: Option<String>,
    clock: Option<CoreClock>,
    visible: bool,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            name: None,
            clock: None,
            visible: false,
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
    pub fn clock(mut self, clock: CoreClock) -> Self {
        self.clock = Some(clock);
        self
    }
    pub fn create(self) -> Result<Event, Error> {
        Event::new(self)
    }
}

pub struct WaitTimeoutResult(bool);

impl WaitTimeoutResult {
    #[must_use]
    pub fn timed_out(&self) -> bool {
        self.0
    }
}

pub struct Event(UnsafeCell<evl_event>);

unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    pub fn new(builder: Builder) -> Result<Self, Error> {
        let this = Self(UnsafeCell::new(unsafe {
            MaybeUninit::<evl_event>::zeroed().assume_init()
        }));
        let mut c_flags = CloneFlags::PRIVATE.bits() as c_int;
        if builder.visible {
            c_flags = CloneFlags::PUBLIC.bits() as c_int;
        }
        let mut c_clockfd = BuiltinClock::MONOTONIC as i32;
        if let Some(clock) = builder.clock {
            c_clockfd = clock.0 as i32;
        }
        let ret: c_int = unsafe {
            if let Some(name) = builder.name {
                let c_name = CString::new(name).expect("CString::new failed");
                let c_fmt = CString::new("%s").expect("CString::new failed");
                evl_create_event(
                    this.0.get(),
                    c_clockfd,
                    c_flags,
                    c_fmt.as_ptr(),
                    c_name.as_ptr(),
                )
            } else {
                evl_create_event(this.0.get(),
                               c_clockfd,
                               c_flags,
                               ptr::null())
            }
        };
        match ret {
            0.. => return Ok(this),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>
    ) -> Result<MutexGuard<'a, T>, Error> {
        let ret: c_int = unsafe {
            evl_wait_event(self.0.get(),
                           guard.as_raw_mut())
        };
        match ret {
            0.. => return Ok(guard),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }

    pub fn wait_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> Result<MutexGuard<'a, T>, Error>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut *guard) {
            guard = self.wait(guard)?;
        }
        Ok(guard)
    }

    pub fn wait_timed<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Instant::<CoreClock>,
    ) -> Result<(MutexGuard<'a, T>, WaitTimeoutResult), Error> {
        let dur = timeout.duration_since_epoch();
        let secs: Seconds<u64> = Seconds::try_from(dur).unwrap();
        let nsecs: Nanoseconds<u64> = Nanoseconds::<u64>::try_from(dur).unwrap() % secs;
        let date = timespec {
            tv_sec: secs.integer() as time_t,
            tv_nsec: nsecs.integer() as c_long,
        };
        let ret: c_int = unsafe {
            evl_timedwait_event(self.0.get(), guard.as_raw_mut(), &date)
        };
        if ret == -ETIMEDOUT {
            return Ok((guard, WaitTimeoutResult(true)));
        }
        match ret {
            0.. => return Ok((guard, WaitTimeoutResult(false))),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }

    pub fn wait_timed_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        timeout: Instant::<CoreClock>,
        mut condition: F,
    ) -> Result<(MutexGuard<'a, T>, WaitTimeoutResult), Error>
    where F: FnMut(&mut T) -> bool
    {
        loop {
            if !condition(&mut *guard) {
                return Ok((guard, WaitTimeoutResult(false)));
            }
            let result = self.wait_timed(guard, timeout)?;
            if result.1.0 {
                return Ok(result);
            }
            guard = result.0;
        }
    }

    pub fn notify_one(&self) {
        let ret: c_int = unsafe { evl_signal_event(self.0.get()) };
        if ret != 0 {
            panic!("notify_one() failed with {}", Error::from_raw_os_error(-ret));
        };
    }

    pub fn notify_all(&self) {
        let ret: c_int = unsafe { evl_broadcast_event(self.0.get()) };
        if ret != 0 {
            panic!("notify_all() failed with {}", Error::from_raw_os_error(-ret));
        };
    }

    pub fn notify_directed(&self, target: &Thread) -> Result<(), Error> {
        let ret: c_int = unsafe { evl_signal_thread(self.0.get(), target.0) };
        match ret {
            0 => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            evl_close_event(self.0.get());
        }
    }
}
