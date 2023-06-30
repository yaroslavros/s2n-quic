#[cfg(loom)]
use crate::testing::loom as core_impl;
#[cfg(not(loom))]
use core as core_impl;

#[cfg(loom)]
use crate::testing::loom as alloc_impl;
#[cfg(not(loom))]
use alloc as alloc_impl;

use super::chunks;
use crate::{
    buffer::ReceiveBuffer,
    frame, stream,
    sync::{
        atomic_waker::AtomicWaker,
        mpsc::{self, Cursors},
    },
    transport,
    varint::VarInt,
};
use alloc_impl::sync::Arc;
use bytes::BytesMut;
use core_impl::{
    cell::UnsafeCell,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

pub mod recv;
