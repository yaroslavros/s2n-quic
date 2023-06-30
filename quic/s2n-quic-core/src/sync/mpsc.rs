// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(loom)]
use crate::testing::loom as core_impl;
#[cfg(not(loom))]
use core as core_impl;

use core_impl::{
    cell::UnsafeCell,
    hint,
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, Ordering},
};
use crossbeam_utils::CachePadded;

// https://www.1024cores.net/home/lock-free-algorithms/queues/intrusive-mpsc-node-based-queue

pub struct Cursors<T: Entry<Link>, Link = ()> {
    head: CachePadded<AtomicPtr<T>>,
    tail: UnsafeCell<*mut T>,
    placeholder: UnsafeCell<T>,
    link: PhantomData<Link>,
}

unsafe impl<T: Entry<Link> + Send, Link> Send for Cursors<T, Link> {}
unsafe impl<T: Entry<Link> + Sync, Link> Sync for Cursors<T, Link> {}

pub trait Entry<Link = ()>: Sized {
    unsafe fn placeholder() -> UnsafeCell<Self>;
    fn next(&self) -> &AtomicPtr<Self>;
    unsafe fn drop_ptr(ptr: NonNull<Self>);
}

/// Returned when a `push` is desynchronized and `pop` should be called again
#[derive(Debug)]
pub struct Desync;

impl<T: Entry<Link>, Link> Cursors<T, Link> {
    #[inline]
    pub unsafe fn new() -> Self {
        let placeholder = T::placeholder();
        let head = AtomicPtr::new(core::ptr::null_mut());
        let head = CachePadded::new(head);
        let tail = UnsafeCell::new(core::ptr::null_mut());
        Self {
            head,
            tail,
            placeholder,
            link: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn init(&self) {
        let ptr = self.placeholder.get();
        *self.tail.get() = ptr;
        self.head.store(ptr, Ordering::Relaxed);
    }

    /// # Safety
    ///
    /// The `entry` should point to a valid entry
    #[inline]
    pub unsafe fn push(&self, entry: NonNull<T>) {
        debug_assert!((*entry.as_ptr()).next().load(Ordering::Relaxed).is_null());
        let prev = self.head.swap(entry.as_ptr(), Ordering::AcqRel);
        debug_assert!(!prev.is_null());
        (*prev).next().store(entry.as_ptr(), Ordering::Release);
    }

    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn pop(&self) -> Option<Result<NonNull<T>, Desync>> {
        let tail = *self.tail.get();
        debug_assert!(!tail.is_null());
        let mut tail = NonNull::new_unchecked(tail);

        let mut next = (*tail.as_ptr())
            .next()
            .swap(core::ptr::null_mut(), Ordering::Acquire);

        if tail.as_ptr() == self.placeholder.get() {
            let next_checked = NonNull::new(next)?;
            *self.tail.get() = next_checked.as_ptr();
            tail = next_checked;
            next = (*tail.as_ptr())
                .next()
                .swap(core::ptr::null_mut(), Ordering::Acquire);
        }

        if let Some(next) = NonNull::new(next) {
            *self.tail.get() = next.as_ptr();
            debug_assert!((*tail.as_ptr()).next().load(Ordering::Relaxed).is_null());
            return Some(Ok(tail));
        }

        let head = self.head.load(Ordering::Acquire);

        if tail.as_ptr() != head {
            return Some(Err(Desync));
        }

        (*self.placeholder.get())
            .next()
            .store(core::ptr::null_mut(), Ordering::Release);
        self.push(NonNull::new_unchecked(self.placeholder.get()));

        next = (*tail.as_ptr())
            .next()
            .swap(core::ptr::null_mut(), Ordering::Acquire);

        let next = NonNull::new(next)?;
        *self.tail.get() = next.as_ptr();

        debug_assert!((*tail.as_ptr()).next().load(Ordering::Relaxed).is_null());

        Some(Ok(tail))
    }

    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn clear(&self) {
        while let Some(ptr) = self.pop_sync(32, || {}) {
            T::drop_ptr(ptr);
        }
    }
}

#[cfg(not(kani))]
impl<T: Entry<Link>, Link> Cursors<T, Link> {
    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn pop_try_sync(&self, spin_count: usize) -> Option<Result<NonNull<T>, Desync>> {
        for _ in 0..spin_count {
            if let Ok(v) = self.pop()? {
                return Some(Ok(v));
            }

            hint::spin_loop();
        }

        Some(Err(Desync))
    }

    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn pop_sync<F: Fn()>(&self, spin_count: usize, on_stall: F) -> Option<NonNull<T>> {
        loop {
            if let Ok(v) = self.pop_try_sync(spin_count)? {
                return Some(v);
            }

            on_stall();
        }
    }
}

#[cfg(kani)]
impl<T: Entry<Link>, Link> Cursors<T, Link> {
    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn pop_try_sync(&self, spin_count: usize) -> Option<Result<NonNull<T>, Desync>> {
        let _ = spin_count;
        self.pop()
    }

    /// # Safety
    ///
    /// Can only be called by a single consumer
    #[inline]
    pub unsafe fn pop_sync<F: Fn()>(&self, spin_count: usize, on_stall: F) -> Option<NonNull<T>> {
        let _ = on_stall;
        Some(self.pop_try_sync(spin_count)?.unwrap())
    }
}

impl<T: Entry<Link>, Link> Drop for Cursors<T, Link> {
    fn drop(&mut self) {
        unsafe {
            self.clear();
        }
    }
}
