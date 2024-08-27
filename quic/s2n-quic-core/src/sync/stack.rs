// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::primitive::{self as atomic, AtomicUsize, Ordering};
use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    fmt,
    mem::{self, MaybeUninit},
    ops::ControlFlow,
    panic::{RefUnwindSafe, UnwindSafe},
};
use crossbeam_utils::{Backoff, CachePadded};

#[cfg(test)]
mod tests;

/// A slot in a stack.
struct Slot<T> {
    /// The current stamp.
    ///
    /// If the stamp equals the tail, this node will be next written to. If it equals head + 1,
    /// this node will be next read from.
    stamp: AtomicUsize,

    /// The value in this slot.
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    #[inline]
    unsafe fn get(&self) -> &T {
        (*self.value.get()).assume_init_ref()
    }

    #[inline]
    unsafe fn take(&self) -> T {
        (*self.value.get()).assume_init_read()
    }

    #[inline]
    unsafe fn drop(&self) {
        (*self.value.get()).assume_init_drop();
    }

    #[inline]
    unsafe fn put(&self, v: T) {
        self.value.get().write(MaybeUninit::new(v));
    }

    #[inline]
    unsafe fn replace(&self, v: T) -> T {
        self.value.get().replace(MaybeUninit::new(v)).assume_init()
    }
}

pub struct Stack<T> {
    /// The head of the stack.
    ///
    /// This value is a "stamp" consisting of an index into the buffer and a lap, but packed into a
    /// single `usize`. The lower bits represent the index, while the upper bits represent the lap.
    head: CachePadded<AtomicUsize>,

    /// The tail of the stack.
    ///
    /// This value is a "stamp" consisting of an index into the buffer and a lap, but packed into a
    /// single `usize`. The lower bits represent the index, while the upper bits represent the lap.
    tail: CachePadded<AtomicUsize>,

    /// The buffer holding slots.
    buffer: Box<[Slot<T>]>,

    /// A stamp with the value of `{ lap: 1, index: 0 }`.
    one_lap: usize,
}

unsafe impl<T: Send> Sync for Stack<T> {}
unsafe impl<T: Send> Send for Stack<T> {}

impl<T> UnwindSafe for Stack<T> {}
impl<T> RefUnwindSafe for Stack<T> {}

impl<T> Stack<T> {
    /// Creates a new bounded stack with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if the capacity is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::<i32>::new(100);
    /// ```
    #[inline]
    pub fn new(cap: usize) -> Self {
        assert!(cap >= 2, "capacity must be at least 2");

        // Head is initialized to `{ lap: 0, index: 0 }`.
        // Tail is initialized to `{ lap: 0, index: 0 }`.
        let head = 0;
        let tail = 0;

        // Allocate a buffer of `cap` slots initialized
        // with stamps.
        let buffer: Box<[Slot<T>]> = (0..cap)
            .map(|i| {
                // Set the stamp to `{ lap: 0, index: i }`.
                Slot {
                    stamp: AtomicUsize::new(i),
                    value: UnsafeCell::new(MaybeUninit::uninit()),
                }
            })
            .collect();

        // One lap is the smallest power of two greater than `cap`.
        let one_lap = (cap + 1).next_power_of_two();

        Self {
            buffer,
            one_lap,
            head: CachePadded::new(AtomicUsize::new(head)),
            tail: CachePadded::new(AtomicUsize::new(tail)),
        }
    }

    /// Returns the capacity of the stack.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::<i32>::new(100);
    ///
    /// assert_eq!(stack.capacity(), 100);
    /// ```
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if the stack is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(100);
    ///
    /// assert!(stack.is_empty());
    /// let _ = stack.push(1);
    /// assert!(!stack.is_empty());
    /// ```
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(100);
    ///
    /// assert!(stack.is_empty());
    /// let _ = stack.push(1);
    /// assert!(!stack.is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        let head = self.head.load(Ordering::SeqCst);
        let tail = self.tail.load(Ordering::SeqCst);

        // Is the tail lagging one lap behind head?
        // Is the tail equal to the head?
        //
        // Note: If the head changes just before we load the tail, that means there was a moment
        // when the channel was not empty, so it is safe to just return `false`.
        tail == head
    }

    /// Returns `true` if the stack is full.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(2);
    ///
    /// assert!(!stack.is_full());
    /// let _ = stack.push(1);
    /// let _ = stack.push(2);
    /// assert!(stack.is_full());
    /// ```
    #[inline]
    pub fn is_full(&self) -> bool {
        let tail = self.tail.load(Ordering::SeqCst);
        let head = self.head.load(Ordering::SeqCst);

        // Is the head lagging one lap behind tail?
        //
        // Note: If the tail changes just before we load the head, that means there was a moment
        // when the stack was not full, so it is safe to just return `false`.
        head.wrapping_add(self.one_lap) == tail
    }

    /// Returns the number of elements in the stack.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(100);
    /// assert_eq!(stack.len(), 0);
    ///
    /// let _ = stack.push(10);
    /// assert_eq!(stack.len(), 1);
    ///
    /// let _ = stack.push(20);
    /// assert_eq!(stack.len(), 2);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        loop {
            // Load the tail, then load the head.
            let tail = self.tail.load(Ordering::SeqCst);
            let head = self.head.load(Ordering::SeqCst);

            // if the tail changed, then try reloading the values to ensure consistency
            if self.tail.load(Ordering::SeqCst) != tail {
                continue;
            }

            let (len, _) = self.calculate_len(head, tail);
            return len;
        }
    }

    /// Pushes an element into the stack, replacing the oldest element if necessary.
    ///
    /// If the stack is full, the oldest element is replaced and returned,
    /// otherwise `None` is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(2);
    ///
    /// assert_eq!(stack.push(10), None);
    /// assert_eq!(stack.push(20), None);
    /// assert_eq!(stack.push(30), Some(10));
    /// assert_eq!(stack.pop(), Some(30));
    /// ```
    #[inline]
    pub fn push(&self, value: T) -> Option<T> {
        self.push_or_else(value, |v, tail, new_tail, slot| {
            let head = tail.wrapping_sub(self.one_lap);
            let new_head = new_tail.wrapping_sub(self.one_lap);

            // Try moving the head.
            if self
                .head
                .compare_exchange_weak(head, new_head, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                // Move the tail.
                self.tail.store(new_tail, Ordering::SeqCst);

                // Swap the previous value.
                let old = unsafe {
                    // SAFETY: cursor positions guarantee this slot is initialized
                    slot.replace(v)
                };

                // Update the stamp.
                slot.stamp.store(tail + 1, Ordering::Release);

                Err(old)
            } else {
                Ok(v)
            }
        })
        .err()
    }

    #[inline]
    fn push_or_else<F>(&self, mut value: T, f: F) -> Result<(), T>
    where
        F: Fn(T, usize, usize, &Slot<T>) -> Result<T, T>,
    {
        let backoff = Backoff::new();
        let mut tail = self.tail.load(Ordering::Relaxed);

        let mut iterations = 0u64;

        loop {
            debug_assert!(iterations < u16::MAX as u64);
            iterations += 1;

            let current = self.cursor(tail);
            let new_tail = self.next_cursor(&current);
            let slot = self.slot(&current);
            let stamp = slot.stamp.load(Ordering::Acquire);
            let target_stamp = tail + 1;

            // If the tail and the stamp match, we may attempt to push.
            if tail == stamp {
                // Try moving the tail.
                match self.tail.compare_exchange_weak(
                    tail,
                    new_tail.value,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // Write the value into the slot and update the stamp.
                        unsafe {
                            // SAFETY: slot positions guarantee this value is uninitialized
                            slot.put(value);
                        }
                        slot.stamp.store(target_stamp, Ordering::Release);
                        return Ok(());
                    }
                    Err(t) => {
                        tail = t;
                        backoff.spin();
                    }
                }
            } else if stamp.wrapping_add(self.one_lap) == target_stamp {
                atomic::fence(Ordering::SeqCst);
                value = f(value, tail, new_tail.value, slot)?;
                backoff.spin();
                tail = self.tail.load(Ordering::Relaxed);
            } else {
                // Snooze because we need to wait for the stamp to get updated.
                backoff.snooze();
                tail = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    /// Attempts to pop an element from the top of the stack.
    ///
    /// If the stack is empty, `None` is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use s2n_quic_core::sync::stack::Stack;
    ///
    /// let stack = Stack::new(2);
    /// assert_eq!(stack.push(10), None);
    /// assert_eq!(stack.push(20), None);
    ///
    /// assert_eq!(stack.pop(), Some(20));
    /// assert_eq!(stack.pop(), Some(10));
    /// assert!(stack.pop().is_none());
    /// ```
    #[inline]
    pub fn pop(&self) -> Option<T> {
        let backoff = Backoff::new();
        let mut tail = self.tail.load(Ordering::Relaxed);

        let mut iterations = 0u64;

        loop {
            debug_assert!(iterations < u16::MAX as u64);
            iterations += 1;

            let current = self.cursor(tail);
            let new_tail = self.prev_cursor(&current);
            let slot = self.slot(&new_tail);
            let stamp = slot.stamp.load(Ordering::Acquire);
            let target_stamp = stamp.wrapping_sub(1);

            if target_stamp == new_tail.value {
                // Try moving the tail.
                match self.tail.compare_exchange_weak(
                    tail,
                    new_tail.value,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // Read the value from the slot and update the stamp.
                        let msg = unsafe {
                            // SAFETY: cursor positions guarantee this value is initialized
                            slot.take()
                        };
                        slot.stamp.store(target_stamp, Ordering::Release);
                        return Some(msg);
                    }
                    Err(t) => {
                        tail = t;
                        backoff.spin();
                        continue;
                    }
                }
            }

            // make sure the target slot matches the current tail value, otherwise it's probably
            // empty
            if stamp != tail {
                atomic::fence(Ordering::SeqCst);
                let head = self.head.load(Ordering::Relaxed);

                // If the tail equals the head, that means the channel is empty.
                if tail == head {
                    return None;
                }

                backoff.spin();
                tail = self.tail.load(Ordering::Relaxed);
                continue;
            }

            // Snooze because we need to wait for the stamp to get updated.
            backoff.snooze();
            tail = self.tail.load(Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn retain_bottom<F: FnMut(&T) -> ControlFlow<bool>>(&self, mut check: F) {
        let backoff = Backoff::new();
        let mut head = self.head.load(Ordering::Relaxed);

        let mut iterations = 0u64;

        loop {
            debug_assert!(iterations < u16::MAX as u64);
            iterations += 1;

            let cursor = self.cursor(head);
            let slot = self.slot(&cursor);
            let stamp = slot.stamp.load(Ordering::Acquire);
            let new = self.next_cursor(&cursor);

            // If the the stamp is ahead of the head by 1, we may attempt to pop.
            if head + 1 == stamp {
                // Try moving the head.
                match self.head.compare_exchange_weak(
                    head,
                    new.value,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        match check(unsafe { slot.get() }) {
                            ControlFlow::Continue(()) => {
                                unsafe {
                                    slot.drop();
                                }

                                slot.stamp
                                    .store(head.wrapping_add(self.one_lap), Ordering::Release);

                                head = new.value;
                                continue;
                            }
                            ControlFlow::Break(false) => {
                                unsafe {
                                    slot.drop();
                                }

                                slot.stamp
                                    .store(head.wrapping_add(self.one_lap), Ordering::Release);

                                return;
                            }
                            ControlFlow::Break(true) => {
                                // revert the head to the initial value
                                self.head.store(head, Ordering::Release);
                                return;
                            }
                        }
                    }
                    Err(h) => {
                        head = h;
                        backoff.spin();
                    }
                }
            } else if stamp == head {
                atomic::fence(Ordering::SeqCst);
                let tail = self.tail.load(Ordering::Relaxed);

                // If the tail equals the head, that means the channel is empty.
                if tail == head {
                    return;
                }

                backoff.spin();
                head = self.head.load(Ordering::Relaxed);
            } else {
                // Snooze because we need to wait for the stamp to get updated.
                backoff.snooze();
                head = self.head.load(Ordering::Relaxed);
            }
        }
    }

    #[inline]
    #[cfg(debug_assertions)]
    fn cursor(&self, value: usize) -> Cursor {
        Cursor {
            value,
            one_lap: self.one_lap,
            cap: self.buffer.len(),
        }
    }

    #[inline]
    #[cfg(not(debug_assertions))]
    fn cursor(&self, value: usize) -> Cursor {
        Cursor { value }
    }

    #[inline]
    fn prev_cursor(&self, cursor: &Cursor) -> Cursor {
        cursor.prev(self.one_lap, self.buffer.len())
    }

    #[inline]
    fn next_cursor(&self, cursor: &Cursor) -> Cursor {
        cursor.next(self.one_lap, self.buffer.len())
    }

    #[inline]
    fn slot<'a>(&'a self, cursor: &Cursor) -> &'a Slot<T> {
        let index = cursor.index(self.one_lap);
        debug_assert!(self.buffer.len() > index);
        unsafe { self.buffer.get_unchecked(index) }
    }

    #[inline]
    fn calculate_len(&self, head: usize, tail: usize) -> (usize, usize) {
        let hix = head & (self.one_lap - 1);
        let tix = tail & (self.one_lap - 1);

        let len = if hix < tix {
            tix - hix
        } else if hix > tix {
            self.buffer.len() - hix + tix
        } else if tail == head {
            0
        } else {
            self.buffer.len()
        };

        (len, hix)
    }
}

impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        if !mem::needs_drop::<T>() {
            return;
        }

        // Get the index of the head.
        let head = *self.head.get_mut();
        let tail = *self.tail.get_mut();

        let (len, hix) = self.calculate_len(head, tail);
        let cap = self.buffer.len();

        // Loop over all slots that hold a message and drop them.
        for i in 0..len {
            // Compute the index of the next slot holding a message.
            let index = if hix + i < cap {
                hix + i
            } else {
                hix + i - cap
            };

            unsafe {
                debug_assert!(index < self.buffer.len());
                let slot = self.buffer.get_unchecked_mut(index);
                (*slot.value.get()).assume_init_drop();
            }
        }
    }
}

impl<T> fmt::Debug for Stack<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("Stack { .. }")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cursor {
    value: usize,
    #[cfg(debug_assertions)]
    one_lap: usize,
    #[cfg(debug_assertions)]
    cap: usize,
}

impl fmt::Debug for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Cursor");
        d.field("value", &self.value);

        #[cfg(debug_assertions)]
        {
            d.field("lap", &(self.lap(self.one_lap) / self.one_lap))
                .field("index", &self.index(self.one_lap));
        }

        d.finish()
    }
}

impl Cursor {
    #[inline]
    fn index(&self, one_lap: usize) -> usize {
        self.value & (one_lap - 1)
    }

    #[inline]
    fn lap(&self, one_lap: usize) -> usize {
        self.value & !(one_lap - 1)
    }

    #[inline]
    fn prev(&self, one_lap: usize, cap: usize) -> Cursor {
        self.with_value(if self.index(one_lap) > 0 {
            // Same lap, decremented index.
            // Set to `{ lap: lap, index: index - 1 }`.
            self.value - 1
        } else {
            // One lap back, index wraps around to cap - 1.
            // Set to `{ lap: lap.wrapping_sub(1), index: cap - 1 }`.
            self.lap(one_lap).wrapping_sub(one_lap) | (cap - 1)
        })
    }

    #[inline]
    fn next(&self, one_lap: usize, cap: usize) -> Cursor {
        self.with_value(if self.index(one_lap) + 1 < cap {
            // Same lap, incremented index.
            // Set to `{ lap: lap, index: index + 1 }`.
            self.value + 1
        } else {
            // One lap forward, index wraps around to zero.
            // Set to `{ lap: lap.wrapping_add(1), index: 0 }`.
            self.lap(one_lap).wrapping_add(one_lap)
        })
    }

    #[inline]
    fn with_value(&self, value: usize) -> Self {
        Self { value, ..*self }
    }
}
