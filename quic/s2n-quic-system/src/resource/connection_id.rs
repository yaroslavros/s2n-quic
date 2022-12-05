use crate::prelude::*;
use s2n_quic_core::connection::{InitialId, LocalId};
use std::collections::HashMap;

pub type Initial = HashMap<InitialId, Entity>;
pub type Local = HashMap<LocalId, Entity>;

pub fn setup<C: Config>(world: &mut World, _c: &mut C) {
    world.insert(Initial::new());
    world.insert(Local::new());
}
