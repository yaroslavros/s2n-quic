use crate::prelude::*;
use s2n_quic_core::crypto;

#[derive(Debug, Component)]
pub struct InitialKey<K: crypto::InitialKey>(K);

#[derive(Debug, Component)]
pub struct InitialHeaderKey<K: crypto::InitialHeaderKey>(K);
