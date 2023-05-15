// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::{
    cell::UnsafeCell,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use s2n_quic_core::{event, io::tx, sync::spsc};

pub trait TxSocket<Packet>: Unpin {
    type State: Unpin;
    type Error;

    fn poll_send(
        &mut self,
        cx: &mut Context,
        state: &mut Self::State,
        packets: (&mut [Packet], &mut [Packet]),
    ) -> Poll<Result<usize, Self::Error>>;
}

pub struct TxFlush<Packet, Socket: TxSocket<Packet>> {
    occupied: spsc::Receiver<Packet>,
    socket: Socket,
    state: Socket::State,
    free: spsc::Sender<Packet>,
}

impl<Packet, Socket: TxSocket<Packet>> TxFlush<Packet, Socket> {
    pub fn new(
        occupied: spsc::Receiver<Packet>,
        socket: Socket,
        state: Socket::State,
        free: spsc::Sender<Packet>,
    ) -> Self {
        Self {
            occupied,
            socket,
            state,
            free,
        }
    }
}

impl<Packet, S> Future for TxFlush<Packet, S>
where
    S: TxSocket<Packet>,
{
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        for _ in 0..10 {
            macro_rules! ready {
                ($expr:expr) => {
                    match $expr {
                        Poll::Ready(Ok(slice)) => slice,
                        Poll::Ready(Err(_)) => return Poll::Ready(()),
                        Poll::Pending => return Poll::Pending,
                    }
                };
            }

            let occupied = this.occupied.poll_slice(cx);
            let free = this.free.poll_slice(cx);

            let mut occupied = ready!(occupied);
            let mut free = ready!(free);

            debug_assert!(occupied.len() <= free.capacity());

            match this.socket.poll_send(cx, &mut this.state, occupied.peek()) {
                Poll::Ready(Ok(mut count)) => {
                    while count > 0 {
                        if let Some(packet) = occupied.pop() {
                            let _ = free.push(packet);
                        }
                        count -= 1;
                    }

                    continue;
                }
                Poll::Ready(Err(err)) => {
                    // TODO possibly emit shut down message
                    let _ = err;
                    return Poll::Ready(());
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        cx.waker().wake_by_ref();

        Poll::Pending
    }
}

pub struct Tx<Packet: 'static> {
    channels: UnsafeCell<Vec<(spsc::Receiver<Packet>, spsc::Sender<Packet>)>>,
    slices: UnsafeCell<
        Vec<(
            spsc::RecvSlice<'static, Packet>,
            spsc::SendSlice<'static, Packet>,
        )>,
    >,
    is_full: bool,
}

impl<Packet: 'static> Tx<Packet> {
    pub fn new(channels: Vec<(spsc::Receiver<Packet>, spsc::Sender<Packet>)>) -> Self {
        let slices = UnsafeCell::new(Vec::with_capacity(channels.len()));
        let channels = UnsafeCell::new(channels);

        Self {
            channels,
            slices,
            is_full: true,
        }
    }
}

impl<Packet> tx::Tx for Tx<Packet>
where
    Packet: crate::message::Message,
{
    type PathHandle = Packet::Handle;
    type Queue = Queue<'static, Packet>;
    type Error = spsc::ClosedError;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // If we didn't fill up the queue then we don't need to poll for capacity
        if !self.is_full {
            return Poll::Pending;
        }

        // poll both channels to make sure we can make progress in both
        let mut is_any_ready = false;

        for (free, occupied) in self.channels.get_mut() {
            let mut is_ready = true;

            macro_rules! ready {
                ($expr:expr) => {
                    match $expr.poll_slice(cx) {
                        Poll::Ready(Ok(_)) => {}
                        Poll::Ready(Err(err)) => return Err(err).into(),
                        Poll::Pending => is_ready = false,
                    }
                };
            }

            ready!(occupied);
            ready!(free);

            is_any_ready |= is_ready;
        }

        if is_any_ready {
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

        let slices = this.slices.get_mut();

        let mut capacity = 0;

        for (free, occupied) in this.channels.get_mut().iter_mut() {
            let mut free = free.slice();
            let mut occupied = occupied.slice();

            // try to synchronize the peer's queues
            let _ = free.sync();
            let _ = occupied.sync();

            if free.is_empty() || occupied.capacity() == 0 {
                continue;
            }

            capacity += free.len().min(occupied.capacity());
            slices.push((free, occupied));
        }

        // update our full status
        this.is_full = slices.is_empty();

        let is_full = &mut this.is_full;

        let mut queue = Queue {
            slices,
            slice_index: 0,
            capacity,
            is_full,
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

pub struct Queue<'a, Packet> {
    slices: &'a mut Vec<(spsc::RecvSlice<'a, Packet>, spsc::SendSlice<'a, Packet>)>,
    slice_index: usize,
    capacity: usize,
    is_full: &'a mut bool,
}

impl<'a, Packet> tx::Queue for Queue<'a, Packet>
where
    Packet: crate::message::Message,
{
    type Handle = Packet::Handle;

    // TODO pull these from the Packet type
    const SUPPORTS_ECN: bool = true;
    const SUPPORTS_FLOW_LABELS: bool = true;

    #[inline]
    fn push<M>(&mut self, message: M) -> Result<tx::Outcome, tx::Error>
    where
        M: tx::Message<Handle = Self::Handle>,
    {
        // if we're at capacity, then return an error
        if self.capacity == 0 {
            return Err(tx::Error::AtCapacity);
        }

        let (free, occupied) = unsafe {
            // Safety: the slice index should always be in bounds
            self.slices.get_unchecked_mut(self.slice_index)
        };

        // take the first free descriptor, we should have at least one item
        let (head, _) = free.peek();
        let packet = &mut head[0];

        let payload_len = packet.tx_write(message)?;

        // make sure we give capacity back to the free queue
        let packet = free.pop().unwrap();
        // push the descriptor on so it can be transmitted
        let result = occupied.push(packet);

        debug_assert!(result.is_ok(), "occupied queue should have capacity");

        // if this slice is at capacity then increment the index
        if free.is_empty() || occupied.capacity() == 0 {
            self.slice_index += 1;
        }

        // check to see if we're full now
        self.capacity -= 1;
        *self.is_full = self.capacity == 0;

        // let the caller know how big the payload was
        let outcome = tx::Outcome {
            len: payload_len,
            index: 0,
        };

        Ok(outcome)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<'a, Packet> Drop for Queue<'a, Packet> {
    #[inline]
    fn drop(&mut self) {
        self.slices.clear();
    }
}
