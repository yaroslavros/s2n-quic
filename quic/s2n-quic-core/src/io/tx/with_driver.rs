// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{event, io::tx};
use core::task::{Context, Poll};

pub struct WithDriver<Driver, Tx> {
    pub(super) driver: Driver,
    pub(super) inner: Tx,
    pub(super) has_capacity: bool,
}

pub trait Driver: Send {
    fn poll_ready(&mut self, has_capacity: bool, cx: &mut Context);
}

impl<D, Tx> tx::Tx for WithDriver<D, Tx>
where
    D: Driver,
    Tx: tx::Tx,
{
    type PathHandle = Tx::PathHandle;
    type Queue = Tx::Queue;
    type Error = Tx::Error;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        self.driver.poll_ready(self.has_capacity, cx);
        self.inner.poll_ready(cx)
    }

    #[inline]
    fn queue<F: FnOnce(&mut Self::Queue)>(&mut self, f: F) {
        self.inner.queue(|queue| {
            use tx::Queue;

            f(queue);
            self.has_capacity = queue.has_capacity();
        })
    }

    #[inline]
    fn handle_error<E: event::EndpointPublisher>(self, error: Self::Error, events: &mut E) {
        self.inner.handle_error(error, events)
    }
}
