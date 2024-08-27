// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::time::Duration;
use s2n_quic_core::time::Timestamp;
use s2n_quic_platform::io::testing::{
    now, primary, rand, spawn, time::delay, Executor, Handle, Model,
};
use tokio::sync::mpsc;

mod histogram;
use histogram::{PrintExt as _, PrintMode};

type Error = Box<dyn 'static + Send + Sync + std::error::Error>;
type Result<T = (), E = Error> = core::result::Result<T, E>;

fn exec<F: FnOnce(&Handle) -> Result<O>, O>(f: F) -> Result {
    let network = Model::default();
    let mut executor = Executor::new(network, 1234);
    let handle = executor.handle().clone();

    executor.enter(|| f(&handle))?;

    executor.run();

    Ok(())
}

#[derive(Debug)]
struct Message {
    time: Timestamp,
    #[allow(dead_code)]
    client: usize,
}

impl Message {
    fn new(client: usize) -> Self {
        Self {
            time: now(),
            client,
        }
    }

    fn process(self, event: Event) -> ProcessedMessage {
        ProcessedMessage {
            message: self,
            event,
            time: now(),
        }
    }
}

struct Socket {
    sender: mpsc::Sender<Message>,
    client: usize,
}

impl Socket {
    pub async fn send(&self) {
        let _ = self.sender.send(Message::new(self.client)).await;
    }
}

struct Network {
    send: mpsc::Sender<Message>,
    recv: mpsc::Receiver<Message>,
    acceptors: Vec<mpsc::Sender<Message>>,
    stats: Stats,
    client: usize,
}

impl Network {
    fn new(
        stats: Stats,
        workers: usize,
        backlog: usize,
        acceptor: impl Fn(mpsc::Receiver<Message>),
    ) -> Self {
        let mut acceptors = vec![];
        for _ in 0..workers {
            let backlog = (backlog + workers - 1) / workers;
            let (send, recv) = mpsc::channel(backlog);
            acceptor(recv);
            acceptors.push(send);
        }

        let (send, recv) = mpsc::channel(backlog);

        Self {
            send,
            recv,
            acceptors,
            stats,
            client: 0,
        }
    }

    fn socket(&mut self) -> Socket {
        let client = self.client;
        self.client += 1;
        Socket {
            sender: self.send.clone(),
            client,
        }
    }

    async fn round_robin(self) {
        let Self {
            send,
            mut recv,
            acceptors,
            stats,
            client: _,
        } = self;
        drop(send);
        let mut idx = 0;
        while let Some(msg) = recv.recv().await {
            if let Err(mpsc::error::TrySendError::Full(msg)) = acceptors[idx].try_send(msg) {
                stats.send(msg.process(Event::NetworkDrop));
            }
            idx += 1;
            if idx == acceptors.len() {
                idx = 0;
            }
        }
    }
}

struct Report {
    send: mpsc::UnboundedSender<ProcessedMessage>,
    recv: mpsc::UnboundedReceiver<ProcessedMessage>,
}

impl Default for Report {
    fn default() -> Self {
        let (send, recv) = mpsc::unbounded_channel();
        Self { send, recv }
    }
}

impl Report {
    fn stats(&self) -> Stats {
        Stats {
            queue: self.send.clone(),
        }
    }

    async fn summarize(self) {
        let Self { send, mut recv } = self;
        drop(send);

        let mut ok = histogram::Histogram::<u64>::new(5).unwrap();
        let mut net_drop = histogram::Histogram::<u64>::new(5).unwrap();
        let mut worker_drop = histogram::Histogram::<u64>::new(5).unwrap();
        let mut prune_drop = histogram::Histogram::<u64>::new(5).unwrap();

        while let Some(msg) = recv.recv().await {
            let elapsed = msg.time - msg.message.time;
            let elapsed_nanos: u64 = elapsed.as_nanos().try_into().unwrap_or(u64::MAX);
            match msg.event {
                Event::NetworkDrop => {
                    let _ = net_drop.record(elapsed_nanos);
                }
                Event::WorkerDrop => {
                    let _ = worker_drop.record(elapsed_nanos);
                }
                Event::PruneDrop => {
                    let _ = prune_drop.record(elapsed_nanos);
                }
                Event::App => {
                    let _ = ok.record(elapsed_nanos);
                }
            }
        }

        let mode = PrintMode::Log {
            start: 1,
            exp: 10.0,
        };

        eprintln!("{}", ok.display(mode).with::<Duration>());
        for (label, hist) in [
            ("NETWORK", net_drop),
            ("WORKER", worker_drop),
            ("PRUNE", prune_drop),
        ] {
            if !hist.is_empty() {
                eprintln!("{label} DROPPED:");
                eprintln!("{}", hist.display(mode).with::<Duration>());
            }
        }
    }
}

#[derive(Clone)]
struct Stats {
    queue: mpsc::UnboundedSender<ProcessedMessage>,
}

impl Stats {
    fn send(&self, msg: ProcessedMessage) {
        let _ = self.queue.send(msg);
    }
}

#[derive(Debug)]
struct ProcessedMessage {
    message: Message,
    event: Event,
    time: Timestamp,
}

#[derive(Debug)]
enum Event {
    NetworkDrop,
    WorkerDrop,
    PruneDrop,
    App,
}

struct Client {
    count: usize,
    delay: Duration,
}

impl Client {
    fn spawn(self, socket: Socket) {
        spawn(async move {
            for _ in 0..self.count {
                socket.send().await;
                let jitter = rand::gen_range(0.0..0.1);
                let variance = Duration::from_secs_f64(self.delay.as_secs_f64() * jitter);
                delay(self.delay + variance).await;
            }
        });
    }
}

mod queue {
    use super::*;
    use std::{
        collections::VecDeque,
        future::poll_fn,
        sync::{Arc, Mutex, Weak},
        task::{Poll, Waker},
    };

    pub struct Queue<T> {
        inner: Arc<Mutex<QueueInner<T>>>,
    }

    impl<T> Clone for Queue<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    impl<T> Queue<T> {
        pub fn new(strategy: Strategy, backlog: usize) -> Self {
            let inner = Arc::new(Mutex::new(QueueInner {
                items: VecDeque::new(),
                waiting: VecDeque::new(),
                strategy,
                backlog,
            }));
            Self { inner }
        }

        pub fn pruner(&self) -> Pruner<T> {
            Pruner(Arc::downgrade(&self.inner))
        }

        fn is_open(&self) -> bool {
            let strong_count = Arc::strong_count(&self.inner);

            strong_count > 1
        }

        pub async fn recv(&self) -> Option<T> {
            poll_fn(|cx| {
                if !self.is_open() {
                    return None.into();
                }

                let mut inner = self.inner.lock().unwrap();

                if let Some(item) = inner.items.pop_front() {
                    return Some(item.unwrap()).into();
                }

                inner.waiting.push_back(cx.waker().clone());

                Poll::Pending
            })
            .await
        }

        pub async fn send(&self, msg: T) -> Option<T> {
            if !self.is_open() {
                return None;
            }

            let mut inner = self.inner.lock().unwrap();

            let strat = if matches!(inner.strategy, Strategy::Random) {
                if rand::gen() {
                    Strategy::Fifo
                } else {
                    Strategy::Lifo
                }
            } else {
                inner.strategy
            };

            let dropped = if inner.items.len() == inner.backlog {
                match strat {
                    Strategy::Fifo => {
                        // drop this message
                        return Some(msg);
                    }
                    Strategy::Lifo | Strategy::LifoPrune => inner.items.pop_back(),
                    Strategy::Random => unreachable!(),
                }
            } else {
                None
            }
            .map(Item::unwrap);

            match strat {
                Strategy::Fifo => inner.items.push_back(Item::new(msg)),
                Strategy::Lifo | Strategy::LifoPrune => inner.items.push_front(Item::new(msg)),
                Strategy::Random => unreachable!(),
            }

            if let Some(waker) = inner.waiting.pop_front() {
                waker.wake();
            }

            dropped
        }
    }

    pub struct Pruner<T>(Weak<Mutex<QueueInner<T>>>);

    impl<T> Pruner<T> {
        pub fn prune(&self, threshold: Duration) -> Option<T> {
            let inner = self.0.upgrade()?;

            let mut inner = inner.lock().unwrap();

            if inner.items.len() <= 1 {
                return None;
            }

            if let Some(item) = inner.items.back() {
                if item.age() < threshold {
                    return None;
                }
            }

            inner.items.pop_back().map(Item::unwrap)
        }
    }

    impl<T> Drop for Pruner<T> {
        fn drop(&mut self) {
            let Some(inner) = self.0.upgrade() else {
                return;
            };
            let Ok(mut inner) = inner.lock() else {
                return;
            };
            while let Some(waker) = inner.waiting.pop_front() {
                waker.wake();
            }
        }
    }

    struct QueueInner<T> {
        items: VecDeque<Item<T>>,
        waiting: VecDeque<Waker>,
        strategy: Strategy,
        backlog: usize,
    }

    struct Item<T> {
        value: T,
        enqueued: Timestamp,
    }

    impl<T> Item<T> {
        fn new(value: T) -> Self {
            Self {
                value,
                enqueued: now(),
            }
        }

        fn age(&self) -> Duration {
            now() - self.enqueued
        }

        fn unwrap(self) -> T {
            self.value
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub enum Strategy {
        Fifo,
        Lifo,
        LifoPrune,
        Random,
    }
}
use queue::Queue;

struct App {
    rate: Duration,
}

impl App {
    fn spawn(self, queue: Queue<Message>, stats: Stats) {
        spawn(async move {
            while let Some(msg) = queue.recv().await {
                stats.send(msg.process(Event::App));
                delay(self.rate).await;
            }
        });
    }
}

fn app_slow(strat: queue::Strategy) {
    exec(|_| {
        use std::sync::Arc;

        let accept_permit = Arc::new(());

        let report = Report::default();
        let stats = report.stats();

        let app_count = 1;
        let queue = Queue::new(strat, 4096);

        for _ in 0..app_count {
            App {
                rate: Duration::from_millis(100),
            }
            .spawn(queue.clone(), stats.clone());
        }

        if matches!(strat, queue::Strategy::LifoPrune) {
            spawn({
                let queue = queue.pruner();
                let stats = stats.clone();
                let accept_permit = accept_permit.clone();
                async move {
                    while Arc::strong_count(&accept_permit) > 1 {
                        loop {
                            match queue.prune(Duration::from_millis(2)) {
                                None => break,
                                Some(dropped) => {
                                    stats.send(dropped.process(Event::PruneDrop));
                                }
                            }
                        }

                        delay(Duration::from_millis(1)).await;
                    }
                }
            });
        }

        let mut network = Network::new(stats.clone(), 1, 10, |mut accept| {
            let queue = queue.clone();
            let stats = stats.clone();
            let accept_permit = accept_permit.clone();
            spawn(async move {
                while let Some(msg) = accept.recv().await {
                    delay(Duration::from_millis(2)).await;
                    if let Some(dropped) = queue.send(msg).await {
                        stats.send(dropped.process(Event::WorkerDrop));
                    }
                }
                drop(accept_permit);
            });
        });

        for _ in 0..17 {
            Client {
                count: 10000,
                delay: Duration::from_millis(100),
            }
            .spawn(network.socket());
        }

        spawn(network.round_robin());

        primary::spawn(async move {
            report.summarize().await;
        });

        Ok(())
    })
    .unwrap();
}

#[test]
fn fifo_app_slow() {
    app_slow(queue::Strategy::Fifo)
}

#[test]
fn lifo_app_slow() {
    app_slow(queue::Strategy::Lifo)
}

#[test]
fn lifo_prune_app_slow() {
    app_slow(queue::Strategy::LifoPrune)
}

#[test]
fn random_app_slow() {
    app_slow(queue::Strategy::Random)
}
