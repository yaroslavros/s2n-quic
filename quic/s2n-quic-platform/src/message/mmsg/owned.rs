// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::message::{
    mmsg::{self, Handle},
    msg::owned::InnerPacket,
    Message,
};
use core::fmt;
use s2n_quic_core::{
    inet::{ExplicitCongestionNotification, SocketAddress},
    io::tx,
};

pub struct Packet(InnerPacket<mmsg::Message>);

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let handle = self.path_handle().unwrap_or(Handle {
            remote_address: Default::default(),
            local_address: Default::default(),
        });
        f.debug_struct("Packet")
            .field("remote_address", &handle.remote_address)
            .field("local_address", &handle.local_address)
            .field("payload_len", &self.payload_len())
            .finish()
    }
}

impl Packet {
    #[inline]
    pub fn new(mtu: u16, max_segments: u16) -> Self {
        Self(InnerPacket::new(mtu, max_segments))
    }

    #[inline]
    pub(crate) fn as_ptr(&mut self) -> *mut libc::mmsghdr {
        &mut self.0.msghdr.0 as *mut _
    }

    #[inline]
    pub(crate) fn as_ref(&self) -> &libc::mmsghdr {
        &self.0.msghdr.0
    }

    #[inline]
    pub(crate) fn as_mut(&mut self) -> &mut libc::mmsghdr {
        &mut self.0.msghdr.0
    }
}

impl_message_delegate!(Packet, libc::mmsghdr, 0.msghdr);
