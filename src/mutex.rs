//! Real-time mutex.
//!
//! EVL provides common mutexes for serializing thread access to a
//! shared resource from [out-of-band
//! context](https://evlproject.org/dovetail/pipeline/#two-stage-pipeline),
//! with semantics close to the [POSIX
//! specification](https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/pthread.h.html).
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

/// A mutex builder `struct` to configure and create a mutex.
pub struct Builder {
    name: Option<String>,
    visible: bool,
    recursive: bool,
    ceiling: u32,
}

impl Builder {
    /// Create a mutex builder. By default, a mutex is unnamed, is not
    /// [visible](https://evlproject.org/core/user-api/#element-visibility)
    /// outside of the current process, is not recursive and enforces
    /// the priority inheritance protocol.
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a visible mutex named 'foo_mutex'.
    /// let builder = Builder::new()
    ///			.name("foo_mutex")
    ///			.visible();
    /// ```
    pub fn new() -> Self {
        Self {
            name: None,
            visible: false,
            recursive: false,
            ceiling: 0,
        }
    }
    /// Set the name property.
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a mutex named 'foo_mutex'.
    /// let builder = Builder::new().name("foo_mutex");
    /// ```
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }
    /// Set the visibility property to 'public', i.e. the mutex is
    /// visible to other processes via its entry into the `/dev/evl`
    /// hierarchy.
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a public mutex.
    /// let builder = Builder::new().public();
    /// ```
    pub fn public(mut self) -> Self {
        self.visible = true;
        self
    }
    /// Set the visibility property to 'private', i.e. the mutex is
    /// not visible to other processes, it has no entry into the
    /// `/dev/evl` hierarchy.
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a private mutex.
    /// let builder = Builder::new().private();
    /// ```
    pub fn private(mut self) -> Self {
        self.visible = false;
        self
    }
    /// Allow the mutex to be taken recursively.
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a recursive mutex.
    /// let builder = Builder::new().recursive();
    /// ```
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }
    /// Set the ceiling value. If non-zero, the priority ceiling
    /// protocol is enabled for the mutex using this value. If zero,
    /// priority inheritance is enabled instead (default).
    ///
    /// ```no_run
    /// use revl::mutex::Builder;
    ///
    /// // A builder for a PCP mutex with ceiling priority at 42.
    /// let builder = Builder::new().ceiling(42);
    /// ```
    pub fn ceiling(mut self, ceiling: u32) -> Self {
        self.ceiling = ceiling;
        self
    }
    /// Create a mutex from the current properties.
    ///
    /// ```no_run
    /// 
    /// ```
    pub fn create<T>(self, data: T) -> Result<Mutex<T>, Error> {
        Mutex::new(data, self)
    }
}

/// The Mutex `struct` implements a mutal exclusion lock.
pub struct Mutex<T: ?Sized> {
    mutex: CoreMutex,
    data: UnsafeCell<T>,
}

unsafe impl<T: Sync + Send> Send for Mutex<T> {}
unsafe impl<T: Sync + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Create a new mutex for guarding `data`, using the properties
    /// defined by the [`builder`](struct@Builder).
    ///
    /// ```no_run
    /// use revl::mutex::Mutex;
    /// 
    /// ```
    pub fn new(data: T, builder: Builder) -> Result<Self, Error> {
        Ok(Self {
            mutex: CoreMutex::new(builder)?,
            data: UnsafeCell::new(data),
        })
    }
    /// Lock the mutex. This call returns an RAII guard which
    /// guarantees exclusive read/write access to the inner data until
    /// such guard goes out of scope, releasing the
    /// mutex. Alternatively, calling [`drop`] on the guard releases
    /// the mutex too.
    ///
    /// ```no_run
    /// use revl::mutex::Mutex;
    /// 
    /// ```
    pub fn lock(&self) -> Result<MutexGuard<T>, Error> {
        self.mutex.lock()?;
        Ok(MutexGuard {
            __mutex: &self.mutex,
            __data: &self.data,
        })
    }
    /// Consume the mutex, returning the inner data.
    ///
    /// ```no_run
    /// use revl::mutex::Mutex;
    ///
    /// let mutex = Mutex::new(42);
    /// assert_eq!(mutex.into_inner(), 42);
    /// ```
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

impl<'a, T: ?Sized> MutexGuard<'a, T> {
    pub(crate) fn as_raw_mut(&self) -> &'a mut evl_mutex {
        unsafe { &mut *self.__mutex.0.get() }
    }
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
