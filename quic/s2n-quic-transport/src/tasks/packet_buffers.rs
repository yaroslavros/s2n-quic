// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub trait PacketBuffer {
    type SegmentSize: SegmentSize;

    fn umem(&self) -> *mut u8;
}

#[derive(Clone, Copy, Debug)]
pub struct Descriptor<SegmentSize> {
    pub address: u32,
    pub len: u32,
    pub segment_size: SegmentSize,
}

impl<SegmentSize> Descriptor<SegmentSize> {
    pub unsafe fn slice(&self, umem: *mut u8) -> &mut [u8] {
        let ptr = umem.add(self.address as _);
        core::slice::from_raw_parts_mut(ptr, self.len as _)
    }
}

pub trait SegmentSize: Clone + Copy + core::fmt::Debug {
    //
}
