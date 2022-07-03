//! Real-time thread.
//!
//! EVL threads are native threads originally, which are extended with
//! real-time capabilities once attached to the EVL core. See [this
//! document](https://evlproject.org/core/user-api/thread/) for an
//! introduction to EVL threads.

use std::thread;
use std::ptr;
use std::os::raw::c_int;
use std::io::Error;
use std::ffi::CString;
use evl_sys::{
    evl_attach_thread,
    evl_unblock_thread,
    evl_demote_thread,
    evl_sched_attrs,
    evl_set_schedattr,
    CloneFlags,
};
use crate::sched::*;

/// A thread factory, which can be used in order to configure the
/// properties of a new EVL thread.
pub struct Builder {
    name: Option<String>,
    visible: bool,
    observable: bool,
    unicast: bool,
}

impl Builder {
    /// Create a thread factory with default settings.
    ///
    /// Methods can be chained on the builder in order to configure
    /// it.
    ///
    /// The available configurations are:
    ///
    /// - `name`: specifies an associated name for the thread
    /// - `visible`: specifies the visibility for the thread in the
    /// [/dev/evl file hierarchy](https://evlproject.org/core/user-api/#evl-fs-hierarchy)
    /// - `observable`: specifies whether the thread may be observed
    /// for health monitoring purpose.
    /// - `unicast`: if observable, specifies whether notifications
    /// should be sent to a single observer instead of broadcast
    /// to all of them.
    pub fn new() -> Self {
        Self {
            name: None,
            visible: false,
            observable: false,
            unicast: false,
        }
    }
    /// Set the thread name. This name must conform to the [naming
    /// convention](https://evlproject.org/core/user-api/#element-naming-convention)
    /// for EVL elements.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }
    /// Set the thread visibility to 'public', as defined by this
    /// [document](https://evlproject.org/core/user-api/#evl-fs-hierarchy).
    ///
    /// ```no_run
    /// use revl::thread;
    /// use std::path;
    ///
    /// let thread = thread::Builder::new().name("foo_thread").public().attach().unwrap();
    /// let path = Path::new("/dev/evl/threads/foo_thread");
    /// assert_eq!(path.exists(), true);
    /// ```
    pub fn public(mut self) -> Self {
        self.visible = true;
        self
    }
    /// Set the thread visibility to 'private', as defined by this
    /// [document](https://evlproject.org/core/user-api/#evl-fs-hierarchy). This
    /// is the default setting.
    ///
    /// ```no_run
    /// use revl::thread;
    /// use std::path;
    ///
    /// let thread = thread::Builder::new().name("bar_thread").private().attach().unwrap();
    /// let path = Path::new("/dev/evl/threads/bar_thread");
    /// assert_eq!(path.exists(), false);
    /// ```
    pub fn private(mut self) -> Self {
        self.visible = false;
        self
    }
    /// Make the thread observable for health monitoring purpose, as
    /// defined by this
    /// [document](https://evlproject.org/core/user-api/observable/#observable-thread).
    pub fn observable(mut self) -> Self {
        self.observable = true;
        self
    }
    /// Enable unicast mode for an observable thread, restricting the
    /// notification of events to a single observer.
    pub fn unicast(mut self) -> Self {
        self.unicast = true;
        self
    }
    /// Attach the calling thread to the EVL core, consuming the
    /// builder.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// thread::Builder::new()
    ///		.name("foo_thread")
    ///		.private()
    ///		.observable()
    ///		.attach().expect("cannot attach thread to EVL core");
    /// ```
    pub fn attach(self) -> Result<Thread, Error> {
        Thread::attach(self)
    }
    /// Spawn a new EVL thread using the current properties, consuming
    /// the builder.
    ///
    /// On success, this call returns a join handle, which implements
    /// the [`join()`][`thread::JoinHandle::join`] method that can be
    /// used to wait for the spawned thread to exit.
    ///
    /// The spawned thread may outlive the caller (unless the caller
    /// thread is the main thread; the whole process is terminated
    /// when the main thread finishes). The join handle can be used to
    /// block on termination of the spawned thread, including
    /// recovering its panics.
    ///
    /// The reason for the `'static + Send` bounds required from the
    /// closure type are explained in the documentation of the
    /// standard [`std::thread::spawn()`][`thread::spawn`] call.
    ///
    /// # Errors
    ///
    /// On error, this call may directly return an error status from
    /// [`std::thread::spawn()`][`thread::spawn`] without starting the
    /// thread. Otherwise, joining the spawned thread may return an
    /// error status related to attaching the thread to the EVL
    /// core. See below.
    ///
    /// ## Join errors
    ///
    /// On error attaching the new thread to the core,
    /// [`join()`][`thread::JoinHandle::join`] may return any of the
    /// following statuses:
    ///
    /// * [`AlreadyExists`][`std::io::ErrorKind`] is returned if an
    /// existing thread already goes by the same name.
    ///
    /// * [`InvalidInput`][`std::io::ErrorKind`] may denote a badly formed
    /// name. Check these
    /// [rules](https://evlproject.org/core/user-api/#element-naming-convention).
    ///
    /// * [`PermissionDenied`][`std::io::ErrorKind`] means that the
    /// calling thread is not allowed to lock memory by a call to
    /// [mlockall(2)](https://man7.org/linux/man-pages/man2/mlock.2.html),
    /// which is a showstopper for real-time execution.
    ///
    /// All other kinds report operating system level errors, see the
    /// complete list from the C interface available from
    /// <https://evlproject.org/core/user-api/thread/#evl_attach_thread>.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// let builder = thread::Builder::new();
    ///
    /// let handle = builder.spawn(|| {
    ///     // your EVL thread code
    /// }).unwrap();
    ///
    /// handle.join().unwrap();
    /// ```
    pub fn spawn<F>(self, f: F) -> Result<thread::JoinHandle<Result<(), Error>>, Error>
    where F: FnOnce() + Send + 'static
    {
        Ok(thread::Builder::new().spawn(move || -> Result<(), Error> {
            self.attach()?;
            Ok(f())
        })?)
    }
}

pub struct Thread(pub(crate) c_int);

unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}

impl Thread {
    /// Attach the calling thread to the EVL core.
    ///
    /// The [`Builder`] struct contains the EVL-specific properties to
    /// use.
    ///
    /// # Errors
    ///
    /// * [`AlreadyExists`][`std::io::ErrorKind`] means the thread
    /// name is conflicting with an existing thread name.
    ///
    /// * [`InvalidInput`][`std::io::ErrorKind`] means that the thread
    /// name contains invalid characters: such name must contain only
    /// valid characters in the context of a Linux file name.
    ///
    /// * [`Other`][`std::io::ErrorKind`] may mean that either the EVL
    /// core is not enabled in the kernel, or there is an ABI mismatch
    /// between the underlying [evl-sys
    /// crate](https://source.denx.de/Xenomai/xenomai4/evl-sys) and
    /// the EVL core. See [these
    /// explanations](https://evlproject.org/core/under-the-hood/abi/)
    /// for the latter.
    ///
    /// * [`PermissionDenied`][`std::io::ErrorKind`] means that the
    /// calling context is not granted the privileges required by the
    /// attachment operation, such as locking memory via the
    /// [mlockall(2)](http://man7.org/linux/man-pages/man2/mlock.2.html)
    /// system call.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// let props = thread::Builder::new().name("foo_thread").public();
    /// thread::Thread::attach(props).expect("cannot attach thread to EVL core");
    /// ```
    pub fn attach(builder: Builder) -> Result<Self, Error> {
	let mut c_flags = CloneFlags::PRIVATE.bits() as c_int;
        if builder.visible {
	    c_flags = CloneFlags::PUBLIC.bits() as c_int;
        }
        if builder.observable {
	    c_flags |= CloneFlags::OBSERVABLE.bits() as c_int;
        }
        if builder.unicast {
	    c_flags |= CloneFlags::UNICAST.bits() as c_int;
        }
	let ret: c_int = unsafe {
            if let Some(name) = builder.name {
	        let c_name = CString::new(name).expect("CString::new failed");
	        let c_fmt = CString::new("%s").expect("CString::new failed");
	        evl_attach_thread(c_flags, c_fmt.as_ptr(), c_name.as_ptr())
            } else {
                // Anonymous thread (has to be private, the core will
                // check this).
	        evl_attach_thread(c_flags, ptr::null())
            }
	};
	// evl_attach_thread() returns a valid file descriptor or -errno.
	match ret {
	    0.. => return Ok(Thread(ret)),
            _ => return Err(Error::from_raw_os_error(-ret)),
	};
    }
    /// Unblock the target thread.
    ///
    /// If the target thread is currently sleeping on some EVL core
    /// system call, such call is forced to fail. As a result, the
    /// target thread wakes up with an interrupted call status on
    /// return.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// fn unblock_some_thread(t: &thread::Thread) -> Result<(), std::io::Error> {
    ///     t.unblock()
    /// }
    /// ```
    pub fn unblock(&self) -> Result<(), Error> {
	    let ret: c_int = unsafe { evl_unblock_thread(self.0) };
	    match ret {
		0 => return Ok(()),
                _ => return Err(Error::from_raw_os_error(-ret)),
	    }
    }
    /// Demote the target thread to in-band context.
    ///
    /// Demoting a thread means to force it out of any real-time
    /// scheduling class, unblock it like unblock() would do, and kick
    /// it out of the out-of-band stage, all in the same move.  See
    /// details and caveat
    /// [here](https://evlproject.org/core/user-api/thread/#evl_demote_thread).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// fn demote_some_thread(t: &thread::Thread) -> Result<(), std::io::Error> {
    ///     t.demote()
    /// }
    /// ```
    pub fn demote(&self) -> Result<(), Error> {
	    let ret: c_int = unsafe { evl_demote_thread(self.0) };
	    match ret {
		0 => return Ok(()),
                _ => return Err(Error::from_raw_os_error(-ret)),
	    }
    }
    /// Set the scheduling attributes of a thread to `param`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use revl::thread;
    ///
    /// fn set_thread_sched(t: &thread::Thread) -> Result<(), std::io::Error> {
    ///     t.set_sched(SchedFifo { prio: 42 })
    /// }
    /// ```
    pub fn set_sched(&self, param: impl PolicyParam) -> Result<(), Error> {
	let c_attrs_ptr: *const evl_sched_attrs = &param.to_attr().0;
	let ret: c_int = unsafe { evl_set_schedattr(self.0, c_attrs_ptr) };
	match ret {
	    0 => return Ok(()),
            _ => return Err(Error::from_raw_os_error(-ret)),
	}
    }
}
