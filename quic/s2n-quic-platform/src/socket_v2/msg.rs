// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::task::{Context, Poll};
use std::os::unix::io::AsRawFd;

pub use crate::message::msg::owned::Packet;

#[cfg(feature = "std")]
impl super::task::TxSocket<Packet> for std::net::UdpSocket {
    type Error = std::io::Error;

    #[inline]
    fn poll_send(
        &mut self,
        cx: &mut Context,
        (a, b): (&mut [Packet], &mut [Packet]),
    ) -> Poll<Result<usize, Self::Error>> {
        let mut sent = 0;

        macro_rules! finish {
            () => {
                if sent > 0 {
                    Poll::Ready(Ok(sent))
                } else {
                    Poll::Pending
                }
            };
        }

        macro_rules! flush {
            ($queue:ident) => {
                for packet in $queue {
                    match libc!(sendmsg(self.as_raw_fd(), packet.as_ref(), 0)) {
                        Ok(_) => {
                            sent += 1;
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            cx.waker().wake_by_ref();

                            return finish!();
                        }
                        Err(_) => {
                            sent += 1;
                        }
                    }
                }
            };
        }

        flush!(a);
        flush!(b);

        finish!()
    }
}

#[cfg(feature = "tokio")]
impl<T> super::task::TxSocket<Packet> for tokio::io::unix::AsyncFd<T>
where
    T: AsRawFd + Unpin,
{
    type Error = std::io::Error;

    #[inline]
    fn poll_send(
        &mut self,
        cx: &mut Context,
        (a, b): (&mut [Packet], &mut [Packet]),
    ) -> Poll<Result<usize, Self::Error>> {
        let mut guard = match self.poll_write_ready(cx) {
            Poll::Ready(Ok(guard)) => guard,
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Pending => return Poll::Pending,
        };

        let result = guard.try_io(|socket| {
            let mut sent = 0;

            macro_rules! flush {
                ($queue:ident) => {
                    for packet in $queue {
                        match libc!(sendmsg(socket.as_raw_fd(), packet.as_ref(), 0)) {
                            Ok(_) => {
                                sent += 1;
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                return Err(err);
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                                cx.waker().wake_by_ref();

                                return Ok(sent);
                            }
                            Err(_err) => {
                                // TODO do we need to do something special for the errors here?
                                sent += 1;
                            }
                        }
                    }
                };
            }

            flush!(a);
            flush!(b);

            Ok(sent)
        });

        match result {
            Ok(Ok(0)) => Poll::Pending,
            Ok(res) => Poll::Ready(res),
            Err(_err) => Poll::Pending,
        }
    }
}
