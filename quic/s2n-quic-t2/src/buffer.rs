use crate::prelude::*;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};
use std::sync::Arc;

#[inline]
pub fn tick(bump: Res<Bump>) {
    bump.buffer.tick();
}

#[derive(Resource)]
pub struct Bump {
    buffer: Buffer,
    unassigned: Range,
}

impl Bump {
    #[inline]
    pub fn new(len: u32) -> Self {
        let buffer = Buffer::new(len);
        let unassigned = Range::new(0, len);
        Self { buffer, unassigned }
    }

    #[inline]
    pub fn tick(&mut self) {
        self.unassigned.generation.tick();
    }

    #[inline]
    pub fn alloc(&mut self, len: usize) -> Option<Heap> {
        let range = self.unassigned.split_to(len)?;
        let buffer = self.buffer.clone();
        Some(Heap { buffer, range })
    }

    #[inline]
    pub fn get<'r>(&self, range: &'r mut Range) -> &'r mut [u8] {
        self.buffer.get(range)
    }
}

#[derive(Component)]
pub struct Heap {
    buffer: Buffer,
    range: Range,
}

impl Heap {
    pub fn aquire(&mut self) -> HeapBump {
        let mut next = self.buffer.generation();

        // make sure the heap hasn't been aquired multiple times in a generation
        self.range.generation.check_gte(&next);

        let unassigned = self.range.internal_clone();

        next.tick();
        self.range.generation = next;

        HeapBump {
            heap: self,
            unassigned,
        }
    }

    #[inline]
    pub fn get<'r>(&self, range: &'r mut Range) -> &'r mut [u8] {
        self.buffer.get(range)
    }
}

pub struct HeapBump<'a> {
    heap: &'a mut Heap,
    unassigned: Range,
}

impl<'a> HeapBump<'a> {
    #[inline]
    pub fn alloc(&mut self, len: usize) -> Option<Range> {
        self.unassigned.split_to(len)
    }

    #[inline]
    pub fn dealloc(&mut self, range: Range) {
        self.unassigned.generation.check(&range.generation);

        // if the range extends the start of the unassigned range, then roll back the previous
        // allocation. Otherwise, wait until the next tick to reclaim it.
        if range.end == self.unassigned.start {
            self.unassigned.start = range.start;
        }
    }

    #[inline]
    pub fn get<'r>(&self, range: &'r mut Range) -> &'r mut [u8] {
        self.heap.buffer.get(range)
    }
}

#[derive(Debug)]
pub struct Range {
    start: u32,
    end: u32,
    generation: Generation,
}

impl Range {
    #[inline]
    fn new(start: u32, end: u32) -> Self {
        Self {
            start,
            end,
            generation: Default::default(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn split_to(&mut self, at: usize) -> Option<Self> {
        if at > self.len() {
            return None;
        }

        let at = self.start + at as u32;

        let other = Self {
            start: self.start,
            end: at,
            generation: self.generation,
        };

        other.check();

        self.start = at;

        self.check();

        Some(other)
    }

    #[inline]
    pub fn split_off(&mut self, at: usize) -> Option<Self> {
        let mut other = self.split_to(at)?;
        core::mem::swap(self, &mut other);
        Some(other)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.start = self.end;
    }

    #[inline]
    fn internal_clone(&self) -> Self {
        Self {
            start: self.start,
            end: self.end,
            generation: self.generation,
        }
    }

    #[inline]
    fn check(&self) {
        debug_assert!(self.start <= self.end);
    }
}

#[derive(Clone)]
struct Buffer(Arc<BufferState>);

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

struct BufferState {
    bytes: UnsafeCell<Vec<u8>>,
    generation: AtomicU64,
}

impl Buffer {
    fn new(len: u32) -> Self {
        let bytes = vec![0u8; len as usize];
        let bytes = UnsafeCell::new(bytes);
        let state = BufferState {
            bytes,
            generation: AtomicU64::new(0),
        };
        Self(Arc::new(state))
    }

    #[inline]
    pub fn get<'r>(&self, range: &'r mut Range) -> &'r mut [u8] {
        range.generation.check(&self.generation());

        let bytes = unsafe { &mut *self.0.bytes.get() };

        unsafe { bytes.get_unchecked_mut(range.start as usize..range.end as usize) }
    }

    #[inline]
    fn tick(&self) {
        #[cfg(debug_assertions)]
        self.0.generation.fetch_add(1, Ordering::Release);
    }

    #[inline]
    fn generation(&self) -> Generation {
        #[cfg(debug_assertions)]
        {
            Generation {
                generation: self.0.generation.load(Ordering::Acquire),
            }
        }
        #[cfg(not(debug_assertions))]
        {
            Generation {}
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct Generation {
    #[cfg(debug_assertions)]
    generation: u64,
}

impl Generation {
    #[inline]
    fn tick(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.generation += 1;
        }
    }

    #[inline]
    fn check_gte(&self, other: &Self) {
        #[cfg(debug_assertions)]
        {
            assert!(self.generation >= other.generation);
        }
    }

    #[inline]
    fn check(&self, other: &Self) {
        #[cfg(debug_assertions)]
        {
            assert_eq!(self.generation, other.generation);
        }
    }
}
