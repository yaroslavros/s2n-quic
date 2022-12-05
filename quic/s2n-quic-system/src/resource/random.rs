use crate::prelude::*;
use s2n_quic_core::random::Generator;

pub type Source = Box<dyn Generator>;

pub fn setup<C: Config>(world: &mut World, c: &mut C) {
    // TODO
}
