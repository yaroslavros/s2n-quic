// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{ring, umem::Umem};
use core::task::{Context, Poll};
use s2n_codec::{Encoder as _, EncoderBuffer};
use s2n_quic_core::{
    event,
    io::tx,
    sync::atomic_waker,
    xdp::{encoder, path},
};

pub use tx::TxExt;

pub struct Channel {
    pub tx: ring::Tx,
    pub tx_waker: atomic_waker::Handle,
    pub completion: ring::Completion,
}

impl Channel {
    #[inline]
    fn acquire(&mut self, cx: &mut Context) -> Option<bool> {
        trace!("acquiring channel capacity");

        let was_empty = self.is_empty();

        for i in 0..2 {
            let count = self.completion.acquire(u32::MAX);
            let count = self.tx.acquire(count).min(count);

            trace!("acquired {count} entries");

            if count > 0 {
                return Some(was_empty);
            }

            if !self.tx_waker.is_open() {
                return None;
            }

            if i > 0 {
                continue;
            }

            if was_empty {
                trace!("registering tx_waker");
                self.tx_waker.register(cx.waker());
                trace!("waking tx_waker");
                self.tx_waker.wake();
            }
        }

        Some(false)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.tx.is_empty() || self.completion.is_empty()
    }

    #[inline]
    fn wake_tx(&mut self) {
        trace!("needs wakeup: {}", self.tx.needs_wakeup());
        if self.tx.needs_wakeup() || self.is_empty() {
            trace!("tx_waker wake");
            self.tx_waker.wake();
        }
    }
}

pub struct Tx {
    channels: Vec<Channel>,
    umem: Umem,
    encoder: encoder::State,
}

impl Tx {
    /// Creates a TX IO interface for an s2n-quic endpoint
    pub fn new(channels: Vec<Channel>, umem: Umem, encoder: encoder::State) -> Self {
        Self {
            channels,
            umem,
            encoder,
        }
    }

    /// Consumes the TX endpoint into the inner channels
    ///
    /// This is used for internal tests only.
    #[cfg(test)]
    pub fn consume(self) -> Vec<Channel> {
        self.channels
    }
}

impl tx::Tx for Tx {
    type PathHandle = path::Tuple;
    type Queue = Queue<'static>;
    type Error = ();

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // poll both channels to make sure we can make progress in both
        let mut is_any_ready = false;
        let mut is_all_closed = true;

        for (idx, channel) in self.channels.iter_mut().enumerate() {
            if let Some(did_become_ready) = channel.acquire(cx) {
                trace!("channel {idx} became ready: {did_become_ready}");

                is_all_closed = false;
                is_any_ready |= did_become_ready;
            } else {
                trace!("channel {idx} closed");
            }
        }

        if is_all_closed {
            return Err(()).into();
        }

        if is_any_ready {
            trace!("tx ready");
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn queue<F: FnOnce(&mut Self::Queue)>(&mut self, f: F) {
        let this: &'static mut Self = unsafe {
            // Safety: As noted in the [transmute examples](https://doc.rust-lang.org/std/mem/fn.transmute.html#examples)
            // it can be used to temporarily extend the lifetime of a reference. In this case, we
            // don't want to use GATs until the MSRV is >=1.65.0, which means `Self::Queue` is not
            // allowed to take generic lifetimes.
            //
            // We are left with using a `'static` lifetime here and encapsulating it in a private
            // field. The `Self::Queue` struct is then borrowed for the lifetime of the `F`
            // function. This will prevent the value from escaping beyond the lifetime of `&mut
            // self`.
            //
            // See https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=9a32abe85c666f36fb2ec86496cc41b4
            //
            // Once https://github.com/aws/s2n-quic/issues/1742 is resolved this code can go away
            core::mem::transmute(self)
        };

        let mut capacity = 0;

        for (idx, channel) in this.channels.iter_mut().enumerate() {
            let len = channel.tx.acquire(u32::MAX);
            let len = channel.completion.acquire(len).min(len);
            trace!("acquired {len} entries for channel {idx}");
            capacity += len as usize;
        }

        let channels = &mut this.channels;
        let umem = &mut this.umem;
        let encoder = &mut this.encoder;

        let mut queue = Queue {
            channels,
            channel_index: 0,
            channel_needs_wake: false,
            capacity,
            umem,
            encoder,
        };
        f(&mut queue);
    }

    #[inline]
    fn handle_error<E: event::EndpointPublisher>(self, _error: Self::Error, _events: &mut E) {
        // The only reason we would be returning an error is if a channel closed. This could either
        // be because the endpoint is shutting down or one of the tasks panicked. Either way, we
        // don't know what the cause is here so we don't have any events to emit.
    }
}

pub struct Queue<'a> {
    channels: &'a mut Vec<Channel>,
    channel_index: usize,
    channel_needs_wake: bool,
    capacity: usize,
    umem: &'a mut Umem,
    encoder: &'a mut encoder::State,
}

impl<'a> tx::Queue for Queue<'a> {
    type Handle = path::Tuple;

    const SUPPORTS_ECN: bool = true;
    const SUPPORTS_FLOW_LABELS: bool = true;

    #[inline]
    fn push<M>(&mut self, mut message: M) -> Result<tx::Outcome, tx::Error>
    where
        M: tx::Message<Handle = Self::Handle>,
    {
        // if we're at capacity, then return an error
        if self.capacity == 0 {
            trace!("at capacity");
            return Err(tx::Error::AtCapacity);
        }

        let channel = loop {
            let channel = if let Some(channel) = self.channels.get_mut(self.channel_index) {
                channel
            } else {
                return Err(tx::Error::AtCapacity);
            };

            if !channel.is_empty() {
                trace!("selecting channel {}", self.channel_index);
                break channel;
            }

            if core::mem::take(&mut self.channel_needs_wake) {
                trace!("waking channel {}", self.channel_index);
                channel.wake_tx();
            }
            self.channel_index += 1;
        };

        let (entries, _) = channel.completion.data();
        let descriptor = entries[0];

        trace!("using descriptor {descriptor:?}");

        let buffer = unsafe {
            // Safety: this descriptor should be unique, assuming the tasks are functioning
            // properly
            self.umem.get_mut(descriptor)
        };

        // create an encoder for the descriptor region
        let mut buffer = EncoderBuffer::new(buffer);

        // write the message to the encoder using the configuration
        let payload_len = encoder::encode_packet(&mut buffer, &mut message, self.encoder)?;

        // take the length that we wrote and create a RxTxDescriptor with it
        let len = buffer.len();
        let descriptor = descriptor.with_len(len as _);

        trace!("packet written to {descriptor:?}");

        // push the descriptor on so it can be transmitted
        channel.tx.data().0[0] = descriptor;

        // make sure we give capacity back to the free queue
        channel.tx.release(1);
        channel.completion.release(1);
        self.channel_needs_wake = true;

        // check to see if we're full now
        self.capacity -= 1;

        // let the caller know how big the payload was
        let outcome = tx::Outcome {
            len: payload_len as _,
            index: 0,
        };

        Ok(outcome)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<'a> Drop for Queue<'a> {
    #[inline]
    fn drop(&mut self) {
        if self.channel_needs_wake {
            if let Some(channel) = self.channels.get_mut(self.channel_index) {
                trace!("waking channel {}", self.channel_index);
                channel.wake_tx();
            }
        }
    }
}
