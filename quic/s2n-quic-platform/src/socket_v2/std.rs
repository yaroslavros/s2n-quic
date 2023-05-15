// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::task::{Context, Poll};
use s2n_quic_core::inet::SocketAddress;

pub struct Packet(Box<InnerPacket>);

struct InnerPacket {
    remote_address: SocketAddress,
    payload: Vec<u8>,
}

#[cfg(feature = "std")]
impl super::task::TxSocket<Packet> for std::net::UdpSocket {
    type State = ();
    type Error = std::io::Error;

    #[inline]
    fn poll_send(
        &mut self,
        cx: &mut Context,
        _state: &mut Self::State,
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
                    let packet = &packet.0;
                    match self.send_to(&packet.payload, packet.remote_address) {
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
impl super::task::TxSocket<Packet> for tokio::net::UdpSocket {
    type State = ();
    type Error = std::io::Error;

    #[inline]
    fn poll_send(
        &mut self,
        cx: &mut Context,
        _state: &mut Self::State,
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
                    let packet = &packet.0;
                    match self.poll_send_to(cx, &packet.payload, packet.remote_address.into()) {
                        Poll::Ready(Ok(_)) => {
                            sent += 1;
                        }
                        Poll::Ready(Err(_)) => {
                            // TODO do we need to do something special for the errors here?
                            sent += 1;
                        }
                        Poll::Pending => {
                            return finish!();
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
