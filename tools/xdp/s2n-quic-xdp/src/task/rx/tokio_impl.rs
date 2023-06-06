// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::Poller;
use crate::{ring, socket};
use core::task::{Context, Poll};

type Fd = tokio::io::unix::AsyncFd<socket::Fd>;

/// Polls read readiness for a tokio socket
#[inline]
fn poll(fd: &Fd, rx: &mut ring::Rx, cx: &mut Context) -> Poll<Result<(), ()>> {
    // query socket readiness through tokio's polling facilities
    match fd.poll_read_ready(cx) {
        Poll::Ready(Ok(mut guard)) => {
            // try to acquire entries for the queue
            let count = rx.acquire(1) as usize;

            trace!("acquired {count} items from RX ring");

            // if we didn't get anything, we need to clear readiness and try again
            if count == 0 {
                guard.clear_ready();
                trace!("clearing socket readiness");
            }

            Poll::Ready(Ok(()))
        }
        Poll::Ready(Err(err)) => {
            trace!("socket returned an error while polling: {err:?}; closing poller");
            return Poll::Ready(Err(()));
        }
        Poll::Pending => {
            trace!("ring out of items; sleeping");
            return Poll::Pending;
        }
    }
}

/// Polling implementation for an asynchronous socket
impl Poller for Fd {
    #[inline]
    fn poll(&mut self, rx: &mut ring::Rx, cx: &mut Context) -> Poll<Result<(), ()>> {
        poll(self, rx, cx)
    }
}

/// Polling implementation for a shared asynchronous socket
impl Poller for std::sync::Arc<Fd> {
    #[inline]
    fn poll(&mut self, rx: &mut ring::Rx, cx: &mut Context) -> Poll<Result<(), ()>> {
        poll(self, rx, cx)
    }
}
