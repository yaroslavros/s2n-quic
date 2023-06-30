use crate::sync::{
    atomic_waker::AtomicWaker,
    mpsc::{self, Cursors},
};

#[cfg(loom)]
use crate::testing::loom as core_impl;
#[cfg(not(loom))]
use core as core_impl;

#[cfg(loom)]
use crate::testing::loom as alloc_impl;
#[cfg(not(loom))]
use alloc as alloc_impl;

use super::stream;
use alloc_impl::sync::Arc;
use core_impl::{cell::UnsafeCell, ptr::NonNull, sync::atomic::AtomicPtr};

pub mod recv {
    use super::*;

    #[derive(Clone)]
    pub struct Producer(Arc<State>);

    pub struct Consumer(Arc<State>);

    impl Consumer {
        //      #[inline]
        //        pub fn poll_pop(&mut self, cx: &mut Context) -> Poll<Option> {}
    }

    struct State {
        cursors: Cursors<Entry>,
        consumer: AtomicWaker,
    }

    struct Entry(stream::recv::Consumer);

    impl mpsc::Entry for Entry {
        unsafe fn placeholder() -> UnsafeCell<Self> {
            todo!()
        }

        fn next(&self) -> &AtomicPtr<Self> {
            todo!()
        }

        unsafe fn drop_ptr(ptr: NonNull<Self>) {
            let _ = ptr;
            todo!()
        }
    }
}

pub mod bidi {
    use super::*;
}
