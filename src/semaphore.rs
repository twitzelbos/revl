//! Real-time counting semaphore.

use std::cell::UnsafeCell;
use std::ffi::CString;
use std::io::Error;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::ptr;
use evl_sys::{
    evl_close_sem,
    evl_create_sem,
    evl_get_sem,
    evl_put_sem,
    evl_sem,
    evl_tryget_sem,
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
    pub fn create(self) -> Result<Semaphore, Error> {
        Semaphore::new(self)
    }
}

pub struct Semaphore(UnsafeCell<evl_sem>);

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

impl Semaphore {
    /// Create an EVL semaphore.
    ///
    /// # Arguments
    ///
    /// * [`builder`]: a builder struct containing the properties of
    /// the new semaphore.
    ///
    /// # Errors
    ///
    /// * Error(AlreadyExists) means the semaphore name is conflicting
    /// with an existing semaphore name.
    ///
    /// * Error(InvalidInput) means that the semaphore name contains
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
    /// use revl::semaphore::{Builder, Semaphore};
    ///
    /// fn create_a_semaphore(initval: u32) -> Result<Semaphore, std::io::Error> {
    ///     let props = Builder::new().name("a_sema4").public().init_value(initval);
    ///     let me = Semaphore::new(props)?;
    ///     Ok(me)
    /// }
    /// ```
    ///
    pub fn new(builder: Builder) -> Result<Self, Error> {
        let this = Self(UnsafeCell::new(unsafe {
            MaybeUninit::<evl_sem>::zeroed().assume_init()
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
                evl_create_sem(
                    this.0.get(),
                    c_clockfd,
                    c_initval,
                    c_flags,
                    c_fmt.as_ptr(),
                    c_name.as_ptr(),
                )
            } else {
                evl_create_sem(this.0.get(),
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
    pub fn get(&self) -> Result<(), Error> {
        let ret: c_int = unsafe { evl_get_sem(self.0.get()) };
        match ret {
            0.. => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
    pub fn tryget(&self) -> bool {
        let ret: c_int = unsafe { evl_tryget_sem(self.0.get()) };
        match ret {
            0 => return true,
            _ => return false,
        };
    }
    pub fn put(&self) -> Result<(), Error> {
        let ret: c_int = unsafe { evl_put_sem(self.0.get()) };
        match ret {
            0.. => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        unsafe {
            evl_close_sem(self.0.get());
        }
    }
}
