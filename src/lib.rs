//! Rust interface to the EVL real-time core.
//!
//! Provides an API to call the services of the Xenomai4 [real-time
//! core](https://evlproject.org/), aka EVL.

pub mod clock;
pub mod mutex;
pub mod sched;
pub mod thread;
pub mod semaphore;
pub mod flags;
