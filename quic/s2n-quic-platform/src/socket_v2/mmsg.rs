// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::message::cmsg;
use core::{cell::UnsafeCell, pin::Pin};
use libc::{iovec, mmsghdr, sockaddr_in6};

pub struct Messages {
    replicated: Vec<ReplicatedMessage>,
    storage: Storage<mmsghdr>,
}

impl Messages {
    pub unsafe fn slice(&self, index: usize, len: usize) -> &mut [mmsghdr] {
        let messages = &mut *self.storage.messages.get();
        let messages = messages.get_unchecked_mut(index..);
        let messages = messages.get_unchecked_mut(..len);
        messages
    }
}

pub struct ReplicatedMessage {
    primary: *mut mmsghdr,
    secondary: *mut mmsghdr,
}

pub struct Storage<M> {
    pub(crate) messages: UnsafeCell<Pin<Box<[M]>>>,

    // this field holds references to allocated payloads, but is never read directly
    #[allow(dead_code)]
    pub(crate) payloads: Pin<Box<[u8]>>,

    // this field holds references to allocated iovecs, but is never read directly
    #[allow(dead_code)]
    pub(crate) headers: Pin<Box<[Header]>>,
}

pub(crate) struct Header {
    iovec: UnsafeCell<iovec>,
    msg_name: UnsafeCell<sockaddr_in6>,
    cmsg: UnsafeCell<[u8; cmsg::MAX_LEN]>,
}
