use crate::prelude::*;
use s2n_quic_core::crypto as c;

pub struct Initial<K: c::InitialKey>(pub K);
pub struct Handshake<K: c::HandshakeKey>(pub K);

pub fn setup<C: Config>(world: &mut World, config: &mut C) {
    //
}
