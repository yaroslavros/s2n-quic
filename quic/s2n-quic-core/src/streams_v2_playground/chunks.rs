#[cfg(loom)]
use crate::testing::loom as core_impl;
#[cfg(not(loom))]
use core as core_impl;

#[cfg(loom)]
use crate::testing::loom as alloc_impl;
#[cfg(not(loom))]
use alloc as alloc_impl;

use crate::sync::mpsc;
use alloc_impl::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core_impl::{
    cell::UnsafeCell, fmt, mem::ManuallyDrop, ops, ptr::NonNull, sync::atomic::AtomicPtr,
};

pub(crate) type Cursors<T, Link = ()> = mpsc::Cursors<State<T>, Link>;

#[derive(Clone)]
pub struct Pool<T: Default>(Arc<PoolState<T>>);

impl<T: Default> Default for Pool<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

struct PoolState<T: Default> {
    cursors: mpsc::Cursors<State<T>>,
}

impl<T: Default> Pool<T> {
    #[inline]
    pub fn new() -> Self {
        unsafe {
            let cursors = mpsc::Cursors::new();
            let state = PoolState { cursors };
            let state = Arc::new(state);
            state.cursors.init();
            Self(state)
        }
    }

    #[inline]
    pub fn take(&mut self) -> Chunk<T> {
        unsafe {
            match self.0.cursors.pop() {
                Some(Ok(v)) => Chunk::from_ptr(v),
                _ => {
                    let mut chunk = Chunk::new();
                    chunk.0.pool = Arc::downgrade(&self.0);
                    chunk
                }
            }
        }
    }
}

#[inline]
pub fn channel<T: Default>() -> (Producer<T>, Consumer<T>) {
    let chunks = unsafe {
        let chunks = Arc::new(mpsc::Cursors::new());
        chunks.init();
        chunks
    };
    let producer = Producer {
        chunks: chunks.clone(),
    };
    let consumer = Consumer { chunks };
    (producer, consumer)
}

#[derive(Clone)]
pub struct Producer<T: Default> {
    chunks: Arc<mpsc::Cursors<State<T>>>,
}

impl<T: Default> Producer<T> {
    #[inline]
    pub fn push(&self, chunk: Chunk<T>) {
        chunk.push_into(&self.chunks);
    }
}

// The Consumer isn't clone since it's single consumer
pub struct Consumer<T: Default> {
    chunks: Arc<mpsc::Cursors<State<T>>>,
}

impl<T: Default> Consumer<T> {
    #[inline]
    #[cfg(feature = "std")]
    pub fn pop(&self) -> Option<Chunk<T>> {
        let ptr = unsafe { self.chunks.pop_sync(4, std::thread::yield_now) }?;
        let chunk = unsafe { Chunk::from_ptr(ptr) };
        Some(chunk)
    }
}

#[derive(Clone)]
pub struct Chunk<T: Default>(ManuallyDrop<Box<State<T>>>);

impl<T: Default + fmt::Debug> fmt::Debug for Chunk<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.chunk.fmt(f)
    }
}

impl<T: Default> Default for Chunk<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Default> Chunk<T> {
    #[inline]
    pub fn new() -> Self {
        Self(ManuallyDrop::new(Box::new(State::new())))
    }

    #[inline]
    pub(crate) unsafe fn from_ptr(ptr: NonNull<State<T>>) -> Self {
        let state = Box::from_raw(ptr.as_ptr());
        Self(ManuallyDrop::new(state))
    }

    #[inline]
    pub(crate) fn push_into<Link>(self, cursors: &Cursors<T, Link>)
    where
        State<T>: mpsc::Entry<Link>,
    {
        unsafe {
            let mut chunk = ManuallyDrop::new(self);
            let ptr = chunk.state_ptr();
            cursors.push(ptr);
        }
    }

    #[inline]
    pub(crate) fn state_ptr(&mut self) -> NonNull<State<T>> {
        unsafe { NonNull::new_unchecked(&mut **self.0) }
    }
}

impl<T: Default> ops::Deref for Chunk<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0.chunk
    }
}

impl<T: Default> ops::DerefMut for Chunk<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.chunk
    }
}

impl<T: Default> Drop for Chunk<T> {
    #[inline]
    fn drop(&mut self) {
        if let Some(pool) = self.0.pool.upgrade() {
            unsafe {
                let ptr = self.state_ptr();
                pool.cursors.push(ptr);
            }
        } else {
            unsafe {
                ManuallyDrop::drop(&mut self.0);
            }
        }
    }
}

pub(crate) struct State<T: Default> {
    chunk: T,
    next: AtomicPtr<Self>,
    pool: Weak<PoolState<T>>,
}

unsafe impl<T: Default + Send> Send for State<T> {}
unsafe impl<T: Default + Sync> Sync for State<T> {}

impl<T: Clone + Default> Clone for State<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            chunk: self.chunk.clone(),
            next: AtomicPtr::new(core::ptr::null_mut()),
            pool: self.pool.clone(),
        }
    }
}

impl<T: Default> State<T> {
    #[inline]
    fn new() -> Self {
        Self {
            chunk: T::default(),
            next: AtomicPtr::new(core::ptr::null_mut()),
            pool: Weak::new(),
        }
    }
}

impl<T: Default> mpsc::Entry for State<T> {
    #[inline]
    unsafe fn placeholder() -> UnsafeCell<Self> {
        UnsafeCell::new(Self::new())
    }

    #[inline]
    fn next(&self) -> &AtomicPtr<Self> {
        &self.next
    }

    #[inline]
    unsafe fn drop_ptr(ptr: NonNull<Self>) {
        let _ = Box::from_raw(ptr.as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use bolero::{check, TypeGenerator};

    type Chunk = Box<[u8; 4]>;
    type ChunkPool = super::Pool<Chunk>;

    #[derive(Clone, Copy, Debug, TypeGenerator)]
    enum Op {
        Push,
        Pop,
    }

    #[cfg(not(kani))]
    type Ops = Vec<Op>;
    #[cfg(kani)]
    type Ops = crate::testing::InlineVec<Op, 2>;

    struct Model {
        pool: ChunkPool,
        send_counter: u32,
        recv_counter: u32,
        producer: super::Producer<Chunk>,
        consumer: super::Consumer<Chunk>,
    }

    impl Model {
        fn new() -> Self {
            let pool = ChunkPool::new();

            let (producer, consumer) = super::channel();
            Self {
                pool,
                send_counter: 0,
                recv_counter: 0,
                producer,
                consumer,
            }
        }

        fn apply_ops(&mut self, ops: &[Op]) -> &mut Self {
            for op in ops {
                self.apply(*op);
            }
            self
        }

        fn apply(&mut self, op: Op) -> &mut Self {
            match op {
                Op::Push => self.push(),
                Op::Pop => self.pop(),
            }
        }

        fn push(&mut self) -> &mut Self {
            let mut chunk = self.pool.take();
            **chunk = self.send_counter.to_le_bytes();
            self.send_counter += 1;

            self.producer.push(chunk);

            self
        }

        fn pop(&mut self) -> &mut Self {
            match self.consumer.pop() {
                Some(chunk) => {
                    assert_eq!(&chunk[..], &self.recv_counter.to_le_bytes());
                    self.recv_counter += 1;
                }
                None => {
                    assert_eq!(self.send_counter, self.recv_counter);
                }
            }

            self
        }

        fn finish(&mut self) {
            while self.send_counter > self.recv_counter {
                self.pop();
            }
        }
    }

    #[test]
    #[cfg_attr(kani, kani::proof, kani::unwind(3), kani::solver(cadical))]
    fn model() {
        check!().with_type::<Ops>().for_each(|ops| {
            Model::new().apply_ops(ops).finish();
        });
    }

    #[test]
    fn ben() {
        use core::sync::atomic::{AtomicU64, Ordering};

        let entries = std::env::var("ENTRIES").map_or(2048usize, |v| v.parse().unwrap());
        let counter = Arc::new(AtomicU64::new(0));
        let (producer, consumer) = super::channel();

        if std::env::var("POOL").is_ok() {
            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    let mut pool = super::Pool::new();
                    while Arc::strong_count(&counter) > 2 {
                        for _ in 0..entries {
                            producer.push(pool.take());
                        }
                        core::hint::spin_loop();
                    }
                }
            });
        } else {
            std::thread::spawn({
                let counter = counter.clone();
                move || {
                    while Arc::strong_count(&counter) > 2 {
                        for _ in 0..entries {
                            producer.push(super::Chunk::<[u8; 16]>::new());
                        }
                        core::hint::spin_loop();
                    }
                }
            });
        }

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

        for _ in 0..10 {
            std::thread::sleep(core::time::Duration::from_secs(1));
            let count = counter.swap(0, Ordering::Relaxed);
            println!("{count}/s");
        }
    }
}
