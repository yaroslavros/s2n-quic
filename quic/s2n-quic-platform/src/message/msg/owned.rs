// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::message::{cmsg, msg::Handle, Message};
use core::{cell::UnsafeCell, fmt, mem::size_of, pin::Pin};
use s2n_quic_core::{
    inet::{ExplicitCongestionNotification, SocketAddress},
    io::tx,
};

pub struct Packet(Box<InnerPacket>);

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Packet")
            .field(
                "handle",
                &self.path_handle().unwrap_or(Handle {
                    remote_address: Default::default(),
                    local_address: Default::default(),
                }),
            )
            .finish()
    }
}

impl Packet {
    #[inline]
    pub fn new(mtu: u16) -> Self {
        Self(Box::new(InnerPacket::new(mtu)))
    }

    #[inline]
    pub(crate) fn as_ref(&mut self) -> &libc::msghdr {
        &self.0.msghdr
    }

    #[inline]
    pub(crate) fn as_mut(&mut self) -> &mut libc::msghdr {
        &mut self.0.msghdr
    }
}

unsafe impl Send for Packet {}
unsafe impl Sync for Packet {}

struct InnerPacket {
    msghdr: libc::msghdr,
    mtu: u16,
    storage: UnsafeCell<Pin<Box<[u8]>>>,
}

#[repr(C)]
struct Header {
    iovec: libc::iovec,
    msg_name: libc::sockaddr_in6,
    cmsg: [u8; cmsg::MAX_LEN],
}

impl InnerPacket {
    pub fn new(mtu: u16) -> Self {
        let mut len = mtu as usize;
        len += size_of::<Header>();

        let storage = vec![0u8; len].into_boxed_slice();
        let mut storage = UnsafeCell::new(Pin::new(storage));

        let mut msghdr = unsafe { core::mem::zeroed::<libc::msghdr>() };

        let ptr: *mut u8 = storage.get_mut().as_mut_ptr();

        let hdr = unsafe { &mut *(ptr as *mut Header) };

        msghdr.msg_iov = &mut hdr.iovec;
        msghdr.msg_iovlen = 1;

        msghdr.msg_name = &mut hdr.msg_name as *mut _ as *mut _;
        msghdr.msg_namelen = 0;

        msghdr.msg_control = &mut hdr.cmsg as *mut _ as *mut _;
        msghdr.msg_controllen = cmsg::MAX_LEN as _;

        hdr.iovec.iov_base = unsafe { ptr.add(size_of::<Header>()) as *mut _ };

        unsafe {
            msghdr.reset(mtu as _);
        }

        Self {
            msghdr,
            mtu,
            storage,
        }
    }
}

impl_message_delegate!(Packet, libc::msghdr, 0.msghdr);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_test() {
        let mut packet = Packet::new(1500);

        dbg!(packet.path_handle());
        assert_eq!(packet.payload_mut().len(), 1500);
        panic!();
    }
}
