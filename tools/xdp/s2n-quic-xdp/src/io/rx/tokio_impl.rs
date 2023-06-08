// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::Driver;
use crate::{ring, socket, syscall};
use core::task::{Context, Poll};

type Fd = tokio::io::unix::AsyncFd<socket::Fd>;

/// Polls read readiness for a tokio socket
#[inline]
fn poll(fd: &Fd, rx: &mut ring::Rx, fill: &mut ring::Fill, cx: &mut Context) -> Option<u32> {
    let mut count = 0;

    for _ in 0..2 {
        count = rx.acquire(u32::MAX);
        count = fill.acquire(count).min(count);

        if count > 0 {
            return Some(count);
        }

        match fd.poll_read_ready(cx) {
            Poll::Ready(Ok(mut guard)) => {
                guard.clear_ready();
                continue;
            }
            Poll::Ready(Err(err)) => {
                let _ = err;
                return None;
            }
            Poll::Pending => return Some(count),
        }
    }

    let _ = syscall::busy_poll(fd.get_ref());

    print!(".");

    cx.waker().wake_by_ref();
    Some(count)
}

/// Polling implementation for an asynchronous socket
impl Driver for Fd {
    #[inline]
    fn poll(&mut self, rx: &mut ring::Rx, fill: &mut ring::Fill, cx: &mut Context) -> Option<u32> {
        poll(self, rx, fill, cx)
    }
}

/// Polling implementation for a shared asynchronous socket
impl Driver for std::sync::Arc<Fd> {
    #[inline]
    fn poll(&mut self, rx: &mut ring::Rx, fill: &mut ring::Fill, cx: &mut Context) -> Option<u32> {
        poll(self, rx, fill, cx)
    }
}
