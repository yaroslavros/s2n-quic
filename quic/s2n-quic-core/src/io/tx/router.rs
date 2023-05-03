// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{event, io::tx, path};
use core::task::{Context, Poll};

pub trait Router {
    type Handle: path::Handle;

    fn route<M, A, B>(
        &mut self,
        message: M,
        a: &mut A,
        b: &mut B,
    ) -> Result<tx::Outcome, tx::Error>
    where
        M: tx::Message<Handle = Self::Handle>,
        A: tx::Queue<Handle = Self::Handle>,
        B: tx::Queue<Handle = Self::Handle>;
}

pub struct WithRouter<R, A, B> {
    pub(super) router: R,
    pub(super) a: A,
    pub(super) b: B,
}

impl<R, A, B> tx::Tx for WithRouter<R, A, B>
where
    R: 'static + Router,
    A: tx::Tx<PathHandle = R::Handle>,
    B: tx::Tx<PathHandle = R::Handle>,
    A::Queue: 'static,
    B::Queue: 'static,
{
    type PathHandle = A::PathHandle;
    type Queue = Queue<'static, R, A::Queue, B::Queue>;
    type Error = Error<A::Error, B::Error>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        let mut is_ready = true;

        macro_rules! ready {
            ($value:expr, $var:ident) => {
                match $value {
                    Poll::Ready(Ok(())) => {}
                    Poll::Ready(Err(err)) => return Err(Error::$var(err)).into(),
                    Poll::Pending => is_ready = false,
                }
            };
        }

        ready!(self.a.poll_ready(cx), A);
        ready!(self.b.poll_ready(cx), B);

        if is_ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn queue<F: FnOnce(&mut Self::Queue)>(&mut self, f: F) {
        let router = &mut self.router;
        let a = &mut self.a;
        let b = &mut self.b;
        a.queue(|a| {
            b.queue(|b| {
                let (router, a, b): (&'static mut _, &'static mut _, &'static mut _) = unsafe {
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
                    (
                        core::mem::transmute(router),
                        core::mem::transmute(a),
                        core::mem::transmute(b),
                    )
                };

                let mut queue = Queue { router, a, b };
                f(&mut queue);
            });
        });
    }

    #[inline]
    fn handle_error<E: event::EndpointPublisher>(self, error: Self::Error, events: &mut E) {
        match error {
            Error::A(error) => self.a.handle_error(error, events),
            Error::B(error) => self.b.handle_error(error, events),
        }
    }
}

pub enum Error<A, B> {
    A(A),
    B(B),
}

pub struct Queue<'a, R, A, B>
where
    R: Router,
    A: super::Queue<Handle = R::Handle>,
    B: super::Queue<Handle = R::Handle>,
{
    router: &'a mut R,
    a: &'a mut A,
    b: &'a mut B,
}

impl<'a, R, A, B> super::Queue for Queue<'a, R, A, B>
where
    R: Router,
    A: super::Queue<Handle = R::Handle>,
    B: super::Queue<Handle = R::Handle>,
{
    type Handle = R::Handle;

    const SUPPORTS_ECN: bool = A::SUPPORTS_ECN || B::SUPPORTS_ECN;
    const SUPPORTS_PACING: bool = A::SUPPORTS_PACING && B::SUPPORTS_PACING;
    const SUPPORTS_FLOW_LABELS: bool = A::SUPPORTS_FLOW_LABELS || B::SUPPORTS_FLOW_LABELS;

    #[inline]
    fn push<M: tx::Message<Handle = Self::Handle>>(
        &mut self,
        message: M,
    ) -> Result<tx::Outcome, tx::Error> {
        self.router.route(message, self.a, self.b)
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.a.capacity().min(self.b.capacity())
    }

    #[inline]
    fn has_capacity(&self) -> bool {
        self.a.has_capacity() && self.b.has_capacity()
    }
}
