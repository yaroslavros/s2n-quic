// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::{future::Future, pin::Pin, task::Context};
use s2n_quic_core::io::tx::with_driver;

#[derive(Default)]
pub struct Driver {
    futures: Vec<Pin<Box<dyn Send + Future<Output = ()>>>>,
}

impl Driver {
    /// Pushes a future that makes progress on the completion queue
    pub fn push<F: 'static + Send + Future<Output = ()>>(&mut self, f: F) -> &mut Self {
        self.futures.push(Box::pin(f));
        self
    }
}

impl with_driver::Driver for Driver {
    #[inline]
    fn poll_ready(&mut self, has_capacity: bool, cx: &mut Context) {
        // Iterate over all of the driver's futures and poll them
        for f in &mut self.futures {
            let _ = Pin::new(f).poll(cx);
        }

        // if we don't have any capacity then we need to wake ourselves up again to try to get more
        if !has_capacity {
            cx.waker().wake_by_ref();
        }
    }
}
