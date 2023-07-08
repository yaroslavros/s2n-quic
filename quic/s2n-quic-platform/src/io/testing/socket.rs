// TODO

use super::{
    message::Message,
    network::{Buffers, PathHandle},
};
use crate::{
    features::Gso,
    message::Message as _,
    socket::{
        ring, task,
        task::{rx, tx},
    },
    syscall::SocketEvents,
};
use core::task::{Context, Poll};
use s2n_quic_core::path::LocalAddress;
use std::io;

pub async fn rx(
    buffers: Buffers,
    handle: LocalAddress,
    producer: ring::Producer<Message>,
) -> io::Result<()> {
    let result = task::Receiver::new(producer, (handle, buffers)).await;
    if let Some(err) = result {
        Err(err)
    } else {
        Ok(())
    }
}

pub async fn tx(
    buffers: Buffers,
    handle: LocalAddress,
    consumer: ring::Consumer<Message>,
    gso: Gso,
) -> io::Result<()> {
    let result = task::Sender::new(consumer, (handle, buffers), gso).await;
    if let Some(err) = result {
        Err(err)
    } else {
        Ok(())
    }
}

impl tx::Socket<Message> for (LocalAddress, Buffers) {
    type Error = io::Error;

    #[inline]
    fn send(
        &mut self,
        cx: &mut Context,
        entries: &mut [Message],
        events: &mut tx::Events,
    ) -> io::Result<()> {
        todo!();
        /*
        for entry in entries {
            let target = (*entry.remote_address()).into();
            let payload = entry.payload_mut();
            match self.poll_send_to(cx, payload, target) {
                Poll::Ready(Ok(_)) => {
                    if events.on_complete(1).is_break() {
                        return Ok(());
                    }
                }
                Poll::Ready(Err(err)) => {
                    if events.on_error(err).is_break() {
                        return Ok(());
                    }
                }
                Poll::Pending => {
                    events.blocked();
                    break;
                }
            }
        }
        */

        Ok(())
    }
}

impl rx::Socket<Message> for (LocalAddress, Buffers) {
    type Error = io::Error;

    #[inline]
    fn recv(
        &mut self,
        cx: &mut Context,
        entries: &mut [Message],
        events: &mut rx::Events,
    ) -> io::Result<()> {
        todo!();
        /*
        for entry in entries {
            let payload = entry.payload_mut();
            let mut buf = io::ReadBuf::new(payload);
            match self.poll_recv_from(cx, &mut buf) {
                Poll::Ready(Ok(addr)) => {
                    unsafe {
                        let len = buf.filled().len();
                        entry.set_payload_len(len);
                    }
                    entry.set_remote_address(&(addr.into()));

                    if events.on_complete(1).is_break() {
                        return Ok(());
                    }
                }
                Poll::Ready(Err(err)) => {
                    if events.on_error(err).is_break() {
                        return Ok(());
                    }
                }
                Poll::Pending => {
                    events.blocked();
                    break;
                }
            }
        }
        */

        Ok(())
    }
}
