// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::sync::{
    primitive::{Arc, AtomicUsize, Ordering},
    stack::Stack,
};
use core::{fmt, marker::PhantomPinned, ops::ControlFlow, pin::Pin, task::Poll};
use event_listener_strategy::{
    easy_wrapper,
    event_listener::{Event, EventListener},
    EventListenerFuture, Strategy,
};
use pin_project_lite::pin_project;

pub fn new<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    assert!(cap >= 1, "capacity must be at least 2");

    let channel = Arc::new(Channel {
        stack: Stack::new(cap),
        recv_ops: Event::new(),
        stream_ops: Event::new(),
        sender_count: AtomicUsize::new(1),
        receiver_count: AtomicUsize::new(1),
    });

    let s = Sender {
        channel: channel.clone(),
    };
    let r = Receiver {
        listener: None,
        channel,
        _pin: PhantomPinned,
    };
    (s, r)
}

struct Channel<T> {
    stack: Stack<T>,
    recv_ops: Event,
    stream_ops: Event,
    sender_count: AtomicUsize,
    receiver_count: AtomicUsize,
}

impl<T> Channel<T> {
    /// Closes the channel and notifies all blocked operations.
    ///
    /// Returns `true` if this call has closed the channel and it was not closed already.
    fn close(&self) -> bool {
        // TODO
        // let did_close = self.stack.close();
        let did_close = true;

        if !did_close {
            return false;
        }

        // Notify all receive and stream operations.
        self.recv_ops.notify(usize::MAX);
        self.stream_ops.notify(usize::MAX);

        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClosedError;

pub struct Sender<T> {
    channel: Arc<Channel<T>>,
}

impl<T> Sender<T> {
    #[inline]
    pub fn send(&self, msg: T) -> Result<Option<T>, ClosedError> {
        // TODO return closed error from stack
        let res = self.channel.stack.push(msg);

        // Notify a blocked receive operation. If the notified operation gets canceled,
        // it will notify another blocked receive operation.
        self.channel.recv_ops.notify_additional(1);

        // Notify all blocked streams.
        self.channel.stream_ops.notify(usize::MAX);

        Ok(res)
    }

    /// Returns `true` if the channel is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.channel.stack.is_empty()
    }

    /// Returns `true` if the channel is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.channel.stack.is_full()
    }

    /// Returns the number of messages in the channel.
    #[inline]
    pub fn len(&self) -> usize {
        self.channel.stack.len()
    }

    /// Returns the channel capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.channel.stack.capacity()
    }

    /// Returns the number of receivers for the channel.
    #[inline]
    pub fn receiver_count(&self) -> usize {
        self.channel.receiver_count.load(Ordering::SeqCst)
    }

    /// Returns the number of senders for the channel.
    #[inline]
    pub fn sender_count(&self) -> usize {
        self.channel.sender_count.load(Ordering::SeqCst)
    }

    /// Returns whether the senders belong to the same channel.
    #[inline]
    pub fn same_channel(&self, other: &Sender<T>) -> bool {
        Arc::ptr_eq(&self.channel, &other.channel)
    }

    #[inline]
    pub fn retain_bottom<F: FnMut(&T) -> ControlFlow<bool>>(&self, f: F) {
        self.channel.stack.retain_bottom(f)
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Decrement the sender count and close the channel if it drops down to zero.
        if self.channel.sender_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.channel.close();
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender {{ .. }}")
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        let count = self.channel.sender_count.fetch_add(1, Ordering::Relaxed);

        // Make sure the count never overflows, even if lots of sender clones are leaked.
        assert!(count < usize::MAX / 2, "too many senders");

        Sender {
            channel: self.channel.clone(),
        }
    }
}

pin_project! {
    /// The receiving side of a channel.
    ///
    /// Receivers can be cloned and shared among threads. When all receivers associated with a channel
    /// are dropped, the channel becomes closed.
    ///
    /// The channel can also be closed manually by calling [`Receiver::close()`].
    ///
    /// Receivers implement the [`Stream`] trait.
    pub struct Receiver<T> {
        // Inner channel state.
        channel: Arc<Channel<T>>,

        // Listens for a send or close event to unblock this stream.
        listener: Option<EventListener>,

        // Keeping this type `!Unpin` enables future optimizations.
        #[pin]
        _pin: PhantomPinned
    }

    impl<T> PinnedDrop for Receiver<T> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();

            // Decrement the receiver count and close the channel if it drops down to zero.
            if this.channel.receiver_count.fetch_sub(1, Ordering::AcqRel) == 1 {
                this.channel.close();
            }
        }
    }
}

impl<T> Receiver<T> {
    /// Attempts to receive a message from the channel.
    ///
    /// If the channel is empty, or empty and closed, this method returns an error.
    #[inline]
    pub fn try_recv(&self) -> Result<Option<T>, ClosedError> {
        // TODO handle closed error
        let res = self.channel.stack.pop();

        Ok(res)
    }

    /// Receives a message from the channel.
    ///
    /// If the channel is empty, this method waits until there is a message.
    ///
    /// If the channel is closed, this method receives a message or returns an error if there are
    /// no more messages.
    #[inline]
    pub fn recv(&self) -> Recv<'_, T> {
        Recv::_new(RecvInner {
            receiver: self,
            listener: None,
            _pin: PhantomPinned,
        })
    }

    /// Returns `true` if the channel is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.channel.stack.is_empty()
    }

    /// Returns `true` if the channel is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.channel.stack.is_full()
    }

    /// Returns the number of messages in the channel.
    #[inline]
    pub fn len(&self) -> usize {
        self.channel.stack.len()
    }

    /// Returns the channel capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.channel.stack.capacity()
    }

    /// Returns the number of receivers for the channel.
    #[inline]
    pub fn receiver_count(&self) -> usize {
        self.channel.receiver_count.load(Ordering::SeqCst)
    }

    /// Returns the number of senders for the channel.
    #[inline]
    pub fn sender_count(&self) -> usize {
        self.channel.sender_count.load(Ordering::SeqCst)
    }

    /// Returns whether the receivers belong to the same channel.
    #[inline]
    pub fn same_channel(&self, other: &Receiver<T>) -> bool {
        Arc::ptr_eq(&self.channel, &other.channel)
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Receiver {{ .. }}")
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Receiver<T> {
        let count = self.channel.receiver_count.fetch_add(1, Ordering::Relaxed);

        // Make sure the count never overflows, even if lots of receiver clones are leaked.
        assert!(count < usize::MAX / 2);

        Receiver {
            channel: self.channel.clone(),
            listener: None,
            _pin: PhantomPinned,
        }
    }
}

easy_wrapper! {
    /// A future returned by [`Receiver::recv()`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct Recv<'a, T>(RecvInner<'a, T> => Result<T, ClosedError>);
    #[cfg(all(feature = "std", not(target_family = "wasm")))]
    pub(crate) wait();
}

pin_project! {
    #[derive(Debug)]
    #[project(!Unpin)]
    struct RecvInner<'a, T> {
        // Reference to the receiver.
        receiver: &'a Receiver<T>,

        // Listener waiting on the channel.
        listener: Option<EventListener>,

        // Keeping this type `!Unpin` enables future optimizations.
        #[pin]
        _pin: PhantomPinned
    }
}

impl<'a, T> EventListenerFuture for RecvInner<'a, T> {
    type Output = Result<T, ClosedError>;

    /// Run this future with the given `Strategy`.
    fn poll_with_strategy<'x, S: Strategy<'x>>(
        self: Pin<&mut Self>,
        strategy: &mut S,
        cx: &mut S::Context,
    ) -> Poll<Result<T, ClosedError>> {
        let this = self.project();

        loop {
            // Attempt to receive a message.
            if let Some(msg) = this.receiver.try_recv()? {
                return Poll::Ready(Ok(msg));
            }

            // Receiving failed - now start listening for notifications or wait for one.
            if this.listener.is_some() {
                // Poll using the given strategy
                ready!(S::poll(strategy, &mut *this.listener, cx));
            } else {
                *this.listener = Some(this.receiver.channel.recv_ops.listen());
            }
        }
    }
}
