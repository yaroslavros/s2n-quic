// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{ring, syscall};
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use s2n_quic_core::sync::atomic_waker;

/// Polls a RX queue for entries and notifies the waker
pub async fn rx<P: Poller>(poller: P, rx: &ring::Rx, waker: atomic_waker::Handle) {
    let rx = rx.driver_clone();
    Rx { poller, rx, waker }.await;
}

#[cfg(feature = "tokio")]
mod tokio_impl;

/// Polls a socket for pending RX items
pub trait Poller: Unpin {
    fn poll(&mut self, rx: &mut ring::Rx, cx: &mut Context) -> Poll<Result<(), ()>>;
}

pub struct BusyPoll;

/// Busy polls a socket
impl Poller for BusyPoll {
    #[inline]
    fn poll(&mut self, rx: &mut ring::Rx, _cx: &mut Context) -> Poll<Result<(), ()>> {
        let _ = syscall::busy_poll(rx.socket());
        Poll::Ready(Ok(()))
    }
}

struct Rx<P: Poller> {
    poller: P,
    rx: ring::Rx,
    waker: atomic_waker::Handle,
}

impl<P: Poller> Future for Rx<P> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        let Self { poller, rx, waker } = self.get_mut();

        trace!("polling rx");

        let mut prev_producer = rx.producer_index();

        for iteration in 0..10 {
            trace!("iteration {iteration}");

            rx.sync();

            if prev_producer != rx.producer_index() {
                trace!("producer index changed");
                if !rx.is_empty() {
                    trace!("waking waker");
                    waker.wake();
                }
                prev_producer = rx.producer_index();
            }

            let result = poller.poll(rx, cx);

            trace!("poll result {result:?}");

            match result {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(())) => return Poll::Ready(()),
                Poll::Pending => return Poll::Pending,
            }
        }

        if !waker.is_open() {
            trace!("waker is closed; shutting down task");
            return Poll::Ready(());
        }

        trace!("waking self");
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
