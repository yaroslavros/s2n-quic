// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{if_xdp::RxTxDescriptor, ring, umem::Umem};
use core::task::{Context, Poll};
use s2n_codec::DecoderBufferMut;
use s2n_quic_core::{
    event,
    inet::datagram,
    io::rx,
    slice::zip,
    sync::atomic_waker,
    xdp::{decoder, path},
};

/// An interface to handle any errors that happen on the RX IO provider
pub trait ErrorLogger: Send {
    /// Called any time the packet could not be decoded.
    ///
    /// Assuming the correct BPF program is loaded, this should never happen. In case it does, the
    /// application should emit an alarm to diagnose the reason why it's happening.
    fn log_invalid_packet(&mut self, bytes: &[u8]);
}

pub struct Channel {
    pub rx: ring::Rx,
    pub rx_waker: atomic_waker::Handle,
    pub fill: ring::Fill,
}

impl Channel {
    #[inline]
    fn acquire(&mut self, cx: &mut Context) -> Option<u32> {
        let mut count = 0;

        // iterate twice to avoid race conditions on the waker registration
        for i in 0..2 {
            count = self.rx.acquire(u32::MAX);
            if count > 0 {
                break;
            }

            // check to see if the channel is open
            if !self.rx_waker.is_open() {
                return None;
            }

            if i > 0 {
                continue;
            }

            trace!("registering rx_waker");
            self.rx_waker.register(cx.waker());
            trace!("waking rx_waker");
            self.rx_waker.wake();
        }

        Some(self.fill.acquire(count).min(count))
    }

    #[inline]
    fn for_each<F: FnMut(RxTxDescriptor)>(&mut self, mut on_packet: F) {
        // one last effort to acquire any packets
        let len = self.rx.acquire(1);
        let len = self.fill.acquire(len);
        if len == 0 {
            return;
        }

        let rx = self.rx.data();
        let rx = [rx.0, rx.1];

        let fill = self.fill.data();
        let mut fill = [fill.0, fill.1];

        let count = zip(&rx, &mut fill, |rx, fill| {
            for (rx, fill) in rx.iter().copied().zip(fill) {
                on_packet(rx);
                // send the descriptor to the fill queue
                *fill = rx.into();
            }
        });

        trace!("releasing {count} descriptors");

        self.rx.release(count as _);
        self.fill.release(count as _);
    }
}

pub struct Rx {
    channels: Vec<Channel>,
    umem: Umem,
    error_logger: Option<Box<dyn ErrorLogger>>,
}

impl Rx {
    /// Creates a RX IO interface for an s2n-quic endpoint
    pub fn new(channels: Vec<Channel>, umem: Umem) -> Self {
        Self {
            channels,
            umem,
            error_logger: None,
        }
    }

    /// Sets the error logger on the RX IO provider
    pub fn with_error_logger(mut self, error_logger: Box<dyn ErrorLogger>) -> Self {
        self.error_logger = Some(error_logger);
        self
    }
}

impl rx::Rx for Rx {
    type PathHandle = path::Tuple;
    type Queue = Queue<'static>;
    type Error = ();

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        // poll both channels to make sure we can make progress in both

        let mut is_any_ready = false;
        let mut is_all_closed = true;

        for (idx, channel) in self.channels.iter_mut().enumerate() {
            if let Some(count) = channel.acquire(cx) {
                trace!("acquired {count} items from channel {idx}");
                is_all_closed = false;
                is_any_ready |= count > 0;
            } else {
                trace!("channel {idx} closed");
            }
        }

        if is_all_closed {
            return Err(()).into();
        }

        if is_any_ready {
            trace!("rx ready");
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

        let channels = &mut this.channels;
        let umem = &mut this.umem;
        let error_logger = &mut this.error_logger;

        let mut queue = Queue {
            channels,
            umem,
            error_logger,
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
    umem: &'a mut Umem,
    error_logger: &'a mut Option<Box<dyn ErrorLogger>>,
}

impl<'a> rx::Queue for Queue<'a> {
    type Handle = path::Tuple;

    #[inline]
    fn for_each<F: FnMut(datagram::Header<Self::Handle>, &mut [u8])>(&mut self, mut on_packet: F) {
        for (idx, channel) in self.channels.iter_mut().enumerate() {
            trace!("draining channel {idx}");

            channel.for_each(|descriptor| {
                trace!("received descriptor {descriptor:?}");

                let buffer = unsafe {
                    // Safety: this descriptor should be unique, assuming the tasks are functioning
                    // properly
                    self.umem.get_mut(descriptor)
                };

                // create a decoder from the descriptor's buffer
                let decoder = DecoderBufferMut::new(buffer);

                // try to decode the packet and emit the result
                match decoder::decode_packet(decoder) {
                    Ok(Some((header, payload))) => {
                        on_packet(header, payload.into_less_safe_slice());
                    }
                    Ok(None) | Err(_) => {
                        // This shouldn't happen. If it does, the BPF program isn't properly validating
                        // packets before they get to userspace.
                        if let Some(error_logger) = self.error_logger.as_mut() {
                            error_logger.log_invalid_packet(buffer);
                        }
                    }
                }
            });
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        false
    }
}
