use core::mem::MaybeUninit;
use evl_sys::{
    evl_sched_attrs,
    SchedPolicy
};

// Other mods may need visibility on evl_sched_attrs (e.g. thread)
pub struct SchedAttrs(pub(crate) evl_sched_attrs);

pub struct SchedFifo {
    prio: i32,
}

pub struct SchedRR {
    prio: i32,
}

pub struct SchedWeak {
    prio: i32,
}

pub struct SchedQuota {
    group: i32,
    prio: i32,
}

pub struct SchedTP {
    part: i32,
    prio: i32,
}

pub trait PolicyParam {
    fn to_attr(&self) -> SchedAttrs;
}

fn get_zero_attrs() -> SchedAttrs {
    SchedAttrs(unsafe { MaybeUninit::<evl_sched_attrs>::zeroed().assume_init() })
}

impl PolicyParam for SchedFifo {
    fn to_attr(&self) -> SchedAttrs {
        let mut x = get_zero_attrs();
        x.0.sched_policy = SchedPolicy::FIFO as i32;
        x.0.sched_priority = self.prio;
        x
    }
}

impl PolicyParam for SchedRR {
    fn to_attr(&self) -> SchedAttrs {
        let mut x = get_zero_attrs();
        x.0.sched_policy = SchedPolicy::RR as i32;
        x.0.sched_priority = self.prio;
        x
    }
}

impl PolicyParam for SchedWeak {
    fn to_attr(&self) -> SchedAttrs {
        let mut x = get_zero_attrs();
        x.0.sched_policy = SchedPolicy::WEAK as i32;
        x.0.sched_priority = self.prio;
        x
    }
}

impl PolicyParam for SchedQuota {
    fn to_attr(&self) -> SchedAttrs {
        let mut x = get_zero_attrs();
        x.0.sched_policy = SchedPolicy::QUOTA as i32;
        x.0.sched_priority = self.prio;
        x.0.sched_u.quota.__sched_group = self.group;
        x
    }
}

impl PolicyParam for SchedTP {
    fn to_attr(&self) -> SchedAttrs {
        let mut x = get_zero_attrs();
        x.0.sched_policy = SchedPolicy::TP as i32;
        x.0.sched_priority = self.prio;
        x.0.sched_u.tp.__sched_partition = self.part;
        x
    }
}
