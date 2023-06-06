// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{ring, syscall};
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use s2n_quic_core::sync::atomic_waker;

pub async fn tx(tx: &ring::Tx, completion: &ring::Completion, waker: atomic_waker::Handle) {
    let tx = tx.driver_clone();
    let completion = completion.driver_clone();
    Tx {
        tx,
        completion,
        waker,
    }
    .await
}

struct Tx {
    tx: ring::Tx,
    completion: ring::Completion,
    waker: atomic_waker::Handle,
}

impl Future for Tx {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        let Self {
            tx,
            completion,
            waker,
        } = self.get_mut();

        trace!("polling tx");

        let mut progress = true;

        for iteration in 0..10 {
            trace!("iteration {}", iteration);

            let tx_needs_wakeup = if tx.is_empty() {
                tx.sync();
                !tx.is_empty() && tx.needs_wakeup()
            } else {
                tx.needs_wakeup()
            };

            let completion_is_empty = if completion.is_empty() {
                completion.sync();
                completion.is_empty()
            } else {
                false
            };

            trace!(
                "tx_needs_wakeup: {tx_needs_wakeup}; completion_is_empty: {completion_is_empty}"
            );

            if tx_needs_wakeup || completion_is_empty {
                let result = syscall::wake_tx(tx.socket());
                trace!("waking socket for progress {result:?}");
                progress = true;
                continue;
            }

            if !waker.is_open() {
                return Poll::Ready(());
            }

            trace!("waking tx waker");
            waker.wake();
            trace!("registering tx waker");
            waker.register(cx.waker());

            if progress {
                progress = false;
            } else {
                trace!("no progress made; returning");
                return Poll::Pending;
            }
        }

        // if we got here, we iterated 10 times and need to yield so we don't consume the event
        // loop too much
        trace!("waking self");
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}
