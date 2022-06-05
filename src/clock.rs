//! Clock interface.

use libc::{
    c_int,
    c_long,
    time_t,
};
use std::io;
use embedded_time::{
    clock,
    duration::{Nanoseconds, Seconds},
    rate::*,
    Clock, Instant,
};
use evl_sys::{
    evl_read_clock,
    evl_sleep_until,
    timespec,
    BuiltinClock
};

pub struct CoreClock(BuiltinClock);

impl Clock for CoreClock {
    type T = u64;
    const SCALING_FACTOR: Fraction = Fraction::new(1, 1_000_000_000); // ns

    fn try_now(&self) -> Result<Instant<Self>, clock::Error> {
        let mut now = timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe { evl_read_clock(self.0 as c_int, &mut now) };
        let now_ns: u64 = now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64;
        Ok(Instant::new(now_ns))
    }
}

impl CoreClock {
    pub fn sleep_until(&self, timeout: Instant<CoreClock>) -> Result<(), io::Error> {
        let dur = timeout.duration_since_epoch();
        let secs: Seconds<u64> = Seconds::try_from(dur).unwrap();
        let nsecs: Nanoseconds<u64> = Nanoseconds::<u64>::try_from(dur).unwrap() % secs;
        let date = timespec {
            tv_sec: secs.integer() as time_t,
            tv_nsec: nsecs.integer() as c_long,
        };
        let ret: c_int = unsafe { evl_sleep_until(self.0 as c_int, &date) };
        match ret {
            0 => return Ok(()),
            _ => return Err(io::Error::from_raw_os_error(-ret)),
        };
    }
    pub fn now(&self) -> Instant<Self> {
        self.try_now().unwrap()
    }
}

/// EVL implements two builtin clocks: monotonic (aka POSIX
/// CLOCK_MONOTONIC) and adjustable wallclock (aka POSIX CLOCK_REALTIME).
pub const STEADY_CLOCK: CoreClock = CoreClock(BuiltinClock::MONOTONIC);
pub const WALL_CLOCK: CoreClock = CoreClock(BuiltinClock::REALTIME);
