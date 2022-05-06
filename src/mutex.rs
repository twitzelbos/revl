//! Real-time mutex.
//!
//! The implementation borrows from
//! [freertos.rs](https://github.com/hashmismatch/freertos.rs),
//! adapted to the libevl call interface.

use std::ffi::CString;
use std::cell::UnsafeCell;
use std::io::Error;
use std::mem::{forget, MaybeUninit};
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;
use std::fmt;
use std::ptr;
use evl_sys::{
    evl_close_mutex,
    evl_create_mutex,
    evl_lock_mutex,
    evl_mutex,
    evl_unlock_mutex,
    BuiltinClock,
    CloneFlags,
    MutexType,
};

pub struct Builder {
    name: Option<String>,
    visible: bool,
    recursive: bool,
    ceiling: u32,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            name: None,
            visible: false,
            recursive: false,
            ceiling: 0,
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
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }
    pub fn ceiling(mut self, ceiling: u32) -> Self {
        self.ceiling = ceiling;
        self
    }
    pub fn create<T>(self, data: T) -> Result<Mutex<T>, Error> {
        Mutex::new(data, self)
    }
}

pub struct Mutex<T: ?Sized> {
    mutex: CoreMutex,
    data: UnsafeCell<T>,
}

unsafe impl<T: Sync + Send> Send for Mutex<T> {}
unsafe impl<T: Sync + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub fn new(data: T, builder: Builder) -> Result<Self, Error> {
        Ok(Self {
            mutex: CoreMutex::new(builder)?,
            data: UnsafeCell::new(data),
        })
    }
    pub fn lock(&self) -> Result<MutexGuard<T>, Error> {
        self.mutex.lock()?;
        Ok(MutexGuard {
            __mutex: &self.mutex,
            __data: &self.data,
        })
    }
    pub fn into_inner(self) -> T {
        unsafe {
            let (mutex, data) = {
                let Self {
                    ref mutex,
                    ref data,
                } = self;
                (ptr::read(mutex), ptr::read(data))
            };
            forget(self);
            drop(mutex);
            data.into_inner()
        }
    }
}

impl<T: ?Sized> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Mutex address: {:?}", self.mutex)
    }
}

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    __mutex: &'a CoreMutex,
    __data: &'a UnsafeCell<T>,
}

impl<'mutex, T: ?Sized> Deref for MutexGuard<'mutex, T> {
    type Target = T;

    fn deref<'a>(&'a self) -> &'a T {
        unsafe { &*self.__data.get() }
    }
}

impl<'mutex, T: ?Sized> DerefMut for MutexGuard<'mutex, T> {
    fn deref_mut<'a>(&'a mut self) -> &'a mut T {
        unsafe { &mut *self.__data.get() }
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.__mutex.unlock();
    }
}

pub struct CoreMutex(UnsafeCell<evl_mutex>);

impl Drop for CoreMutex {
    fn drop(&mut self) {
        unsafe {
            evl_close_mutex(self.0.get());
        }
    }
}

impl fmt::Debug for CoreMutex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", &self.0 as *const _)
    }
}

impl CoreMutex {
    pub fn new(builder: Builder) -> Result<Self, Error> {
        let this = Self(UnsafeCell::new(unsafe {
            MaybeUninit::<evl_mutex>::zeroed().assume_init()
        }));
        let mut c_flags = CloneFlags::PRIVATE.bits() as c_int;
        if builder.visible {
            c_flags = CloneFlags::PUBLIC.bits() as c_int;
        }
        if builder.recursive {
            c_flags |= MutexType::RECURSIVE.bits() as c_int;
        }
        let c_ceiling = builder.ceiling;
        // Revisit: clock should be configurable.
        let c_clockfd = BuiltinClock::MONOTONIC as i32;
        let ret: c_int = unsafe {
            if let Some(name) = builder.name {
                let c_name = CString::new(name).expect("CString::new failed");
                let c_fmt = CString::new("%s").expect("CString::new failed");
                evl_create_mutex(
                    this.0.get(),
                    c_clockfd,
                    c_ceiling,
                    c_flags,
                    c_fmt.as_ptr(),
                    c_name.as_ptr())
            } else {
                evl_create_mutex(
                    this.0.get(),
                    c_clockfd,
                    c_ceiling,
                    c_flags,
                    ptr::null())
            }
        };
        match ret {
            0.. => return Ok(this),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
    pub fn lock(&self) -> Result<(), Error> {
        let ret: c_int = unsafe { evl_lock_mutex(self.0.get()) };
        match ret {
            0 => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
        };
    }
    pub fn unlock(&self) {
        unsafe {
            evl_unlock_mutex(self.0.get());
        };
    }
}
