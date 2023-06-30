// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::sync::cursor;
use alloc::sync::Arc;
use core::{
    alloc::Layout,
    marker::PhantomData,
    mem::{size_of, MaybeUninit},
    ptr::NonNull,
    sync::atomic::AtomicU32,
};
use crossbeam_utils::CachePadded;

const CURSOR_SIZE: usize = size_of::<CachePadded<AtomicU32>>();
const PRODUCER_OFFSET: usize = 0;
const CONSUMER_OFFSET: usize = CURSOR_SIZE;
const DATA_OFFSET: usize = CURSOR_SIZE * 2;

#[inline]
pub fn channel<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    debug_assert!(capacity.is_power_of_two());

    let entries = capacity as u32;
    let storage = Storage::new(entries);

    let storage = Arc::new(storage);

    let consumer = Consumer {
        cursor: unsafe { storage.builder(entries).build_consumer() },
        storage: storage.clone(),
    };

    let producer = Producer {
        cursor: unsafe { storage.builder(entries).build_producer() },
        storage,
    };

    (producer, consumer)
}

pub struct Producer<T> {
    cursor: cursor::Cursor<MaybeUninit<T>>,
    #[allow(dead_code)]
    storage: Arc<Storage<T>>,
}

unsafe impl<T: Send> Send for Producer<T> {}
unsafe impl<T: Sync> Sync for Producer<T> {}

impl<T> Producer<T> {
    #[inline]
    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.cursor.acquire_producer(1) == 0 {
            return Err(value);
        }

        let (head, _) = unsafe {
            // SAFETY: only called from the Producer
            self.cursor.producer_data()
        };

        unsafe {
            crate::assume!(!head.is_empty());
            head[0].write(value);
        }

        self.cursor.release_producer(1);

        Ok(())
    }

    #[inline]
    pub fn push_iter<I>(&mut self, iter: I) -> usize
    where
        I: IntoIterator<Item = T>,
    {
        let iter = iter.into_iter();
        let mut count = 0;
        let iter = iter.inspect(|_| {
            count += 1;
        });
        self.extend(iter);
        count
    }
}

impl<T> core::iter::Extend<T> for Producer<T> {
    #[inline]
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = T>,
    {
        let mut iter = iter.into_iter();

        let (_lower, upper) = iter.size_hint();

        let watermark = match upper {
            None => u32::MAX,
            Some(v) => v.min(u32::MAX as _) as u32,
        };

        if self.cursor.acquire_producer(watermark) == 0 {
            return;
        }

        let (head, tail) = unsafe {
            // SAFETY: only called from the producer
            self.cursor.producer_data()
        };

        let mut count = 0;

        for slice in [head, tail] {
            for (entry, value) in slice.iter_mut().zip(&mut iter) {
                entry.write(value);
                count += 1;
            }
        }

        self.cursor.release_producer(count);
    }
}

pub struct Consumer<T> {
    cursor: cursor::Cursor<MaybeUninit<T>>,
    #[allow(dead_code)]
    storage: Arc<Storage<T>>,
}

unsafe impl<T: Send> Send for Consumer<T> {}
unsafe impl<T: Sync> Sync for Consumer<T> {}

impl<T> Consumer<T> {
    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.cursor.acquire_consumer(1) == 0 {
            return None;
        }

        let (head, _) = unsafe {
            // SAFETY: only called on the consumer
            self.cursor.consumer_data()
        };

        let value = unsafe {
            crate::assume!(!head.is_empty());
            head[0].assume_init_read()
        };

        self.cursor.release_consumer(1);

        Some(value)
    }

    #[inline]
    pub fn drain(&mut self) -> Drain<T> {
        self.cursor.acquire_consumer(u32::MAX);
        Drain {
            consumer: self,
            index: 0,
            head: true,
            count: 0,
        }
    }
}

pub struct Drain<'a, T> {
    consumer: &'a mut Consumer<T>,
    index: usize,
    head: bool,
    count: u32,
}

impl<'a, T> Iterator for Drain<'a, T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (head, tail) = unsafe {
            // SAFETY: only called on the consumer
            self.consumer.cursor.consumer_data()
        };

        if self.head {
            match head.get(self.index) {
                Some(v) => {
                    self.index += 1;
                    self.count += 1;
                    return Some(unsafe { v.assume_init_read() });
                }
                None => {
                    self.head = false;
                    self.index = 0;
                }
            }
        }

        let v = tail.get(self.index)?;
        self.index += 1;
        self.count += 1;
        Some(unsafe { v.assume_init_read() })
    }
}

impl<'a, T> Drop for Drain<'a, T> {
    #[inline]
    fn drop(&mut self) {
        if self.count > 0 {
            self.consumer.cursor.release_consumer(self.count);
        }
    }
}

/// Tracks allocations of message ring buffer state
pub struct Storage<T> {
    ptr: NonNull<u8>,
    layout: Layout,
    t: PhantomData<T>,
}

impl<T> Storage<T> {
    #[inline]
    pub fn new(entries: u32) -> Self {
        unsafe {
            let layout = Self::layout(entries);
            let ptr = alloc::alloc::alloc_zeroed(layout);
            let ptr = NonNull::new(ptr).expect("could not allocate message storage");
            Self {
                layout,
                ptr,
                t: PhantomData,
            }
        }
    }

    fn layout(entries: u32) -> Layout {
        let cursor = Layout::array::<u8>(DATA_OFFSET).unwrap();
        let entries = Layout::array::<T>((entries) as usize).unwrap();
        let (layout, _entry_offset) = cursor.extend(entries).unwrap();
        layout
    }

    #[inline]
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    #[inline]
    unsafe fn builder(&self, size: u32) -> cursor::Builder<MaybeUninit<T>> {
        let ptr = self.as_ptr();
        let producer = ptr.add(PRODUCER_OFFSET) as *mut _;
        let producer = NonNull::new(producer).unwrap();
        let consumer = ptr.add(CONSUMER_OFFSET) as *mut _;
        let consumer = NonNull::new(consumer).unwrap();
        let data = ptr.add(DATA_OFFSET) as *mut _;
        let data = NonNull::new(data).unwrap();

        cursor::Builder {
            producer,
            consumer,
            data,
            size,
        }
    }
}

impl<T> Drop for Storage<T> {
    fn drop(&mut self) {
        unsafe {
            // Safety: pointer was allocated with self.layout
            alloc::alloc::dealloc(self.as_ptr(), self.layout)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benches() {
        use core::sync::atomic::{AtomicU64, Ordering};

        let counter = Arc::new(AtomicU64::new(0));
        let entries = std::env::var("ENTRIES").map_or(2048usize, |v| v.parse().unwrap());
        let (mut producer, mut consumer) = super::channel(entries);

        if std::env::var("ITER").is_ok() {
            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    while Arc::strong_count(&counter) > 2 {
                        producer.extend(core::iter::repeat([0u8; 16]));
                        core::hint::spin_loop();
                    }
                }
            });

            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    while Arc::strong_count(&counter) > 2 {
                        let count = consumer.drain().count();
                        counter.fetch_add(count as _, Ordering::Relaxed);
                        core::hint::spin_loop();
                    }
                }
            });
        } else {
            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    while Arc::strong_count(&counter) > 2 {
                        while producer.push([0u8; 16]).is_ok() {}
                        core::hint::spin_loop();
                    }
                }
            });

            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    while Arc::strong_count(&counter) > 2 {
                        let mut count = 0;
                        while consumer.pop().is_some() {
                            count += 1;
                        }
                        counter.fetch_add(count, Ordering::Relaxed);
                        core::hint::spin_loop();
                    }
                }
            });
        }

        for _ in 0..10 {
            std::thread::sleep(core::time::Duration::from_secs(1));
            let count = counter.swap(0, Ordering::Relaxed);
            println!("{count}/s");
        }
    }
}
