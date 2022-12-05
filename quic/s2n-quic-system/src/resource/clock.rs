use crate::prelude::*;
use core::time::Duration;
use s2n_quic_core::time::Timestamp;

pub fn setup<C: Config>(world: &mut World, _c: &mut C) {
    world.insert(unsafe { Timestamp::from_duration(Duration::from_micros(1)) });
}
