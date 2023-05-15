// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::message::{
    cmsg,
    msg::{self, Handle},
    Message,
};
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};
use s2n_quic_core::{
    inet::{ExplicitCongestionNotification, SocketAddress},
    io::tx,
};

pub struct Packet(InnerPacket<msg::Message>);

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
    pub(crate) fn as_ref(&mut self) -> &libc::msghdr {
        &self.0.msghdr.0
    }

    #[inline]
    pub(crate) fn as_mut(&mut self) -> &mut libc::msghdr {
        &mut self.0.msghdr.0
    }
}

impl_message_delegate!(Packet, libc::msghdr, 0.msghdr);

pub(crate) struct InnerPacket<M>(NonNull<Header<M>>);

unsafe impl<M: Send> Send for InnerPacket<M> {}
unsafe impl<M: Sync> Sync for InnerPacket<M> {}

impl<M> InnerPacket<M>
where
    M: Default + AsMut<libc::msghdr>,
{
    #[inline]
    pub fn new(mtu: u16, max_segments: u16) -> Self {
        let max_payload_size = mtu * max_segments;
        InnerPacket(Header::<M>::alloc(max_payload_size).expect("failed to allocate packet"))
    }
}

impl<M> Deref for InnerPacket<M> {
    type Target = Header<M>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl<M> DerefMut for InnerPacket<M> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.0.as_mut() }
    }
}

impl<M> Drop for InnerPacket<M> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            let max_payload_size = self.max_payload_size;
            let ptr = self.0.as_ptr() as *mut u8;
            let (layout, _offset) = Header::<M>::layout_unchecked(max_payload_size);
            alloc::alloc::dealloc(ptr, layout)
        }
    }
}

pub(crate) struct Header<M> {
    pub msghdr: M,
    max_payload_size: u16,
    iovec: UnsafeCell<libc::iovec>,
    msg_name: UnsafeCell<libc::sockaddr_in6>,
    cmsg: UnsafeCell<[u8; cmsg::MAX_LEN]>,
}

impl<M> Header<M>
where
    M: Default + AsMut<libc::msghdr>,
{
    #[inline]
    pub fn alloc(max_payload_size: u16) -> Option<NonNull<Self>> {
        unsafe {
            let (layout, data_offset) = Self::layout(max_payload_size).ok()?;
            let header = alloc::alloc::alloc(layout);
            let header = header as *mut Self;
            let header = NonNull::new(header)?;

            header.as_ptr().write(Self::init(max_payload_size));

            {
                let hdr = &mut *header.as_ptr();

                let msghdr = hdr.msghdr.as_mut();

                msghdr.msg_iov = hdr.iovec.get();
                msghdr.msg_iovlen = 1;

                msghdr.msg_name = hdr.msg_name.get() as *mut _;
                msghdr.msg_namelen = 0;

                msghdr.msg_control = hdr.cmsg.get() as *mut _;
                msghdr.msg_controllen = cmsg::MAX_LEN as _;

                hdr.iovec.get_mut().iov_base = header.as_ptr().add(data_offset) as *mut _;

                msghdr.reset(0);
            }

            Some(header)
        }
    }
    #[inline]
    fn init(max_payload_size: u16) -> Self {
        Self {
            msghdr: Default::default(),
            max_payload_size,
            iovec: UnsafeCell::new(unsafe { core::mem::zeroed() }),
            msg_name: UnsafeCell::new(unsafe { core::mem::zeroed() }),
            cmsg: UnsafeCell::new([0; cmsg::MAX_LEN]),
        }
    }
}

impl<M> Header<M> {
    #[inline]
    fn layout(max_payload_size: u16) -> Result<(Layout, usize), alloc::alloc::LayoutError> {
        let header_layout = Layout::new::<Self>();
        let data_layout = Layout::array::<u8>(max_payload_size as usize)?;
        let (layout, data_offset) = header_layout.extend(data_layout)?;
        Ok((layout, data_offset))
    }

    #[inline]
    unsafe fn layout_unchecked(max_payload_size: u16) -> (Layout, usize) {
        if let Ok(v) = Self::layout(max_payload_size) {
            v
        } else {
            core::hint::unreachable_unchecked()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_test() {
        assert_eq!(core::mem::size_of::<libc::mmsghdr>(), 1);
        let mut packet = Packet::new(1500, 1);

        dbg!(&packet);
        assert_eq!(packet.payload_mut().len(), 1500);
        panic!();
    }
}
