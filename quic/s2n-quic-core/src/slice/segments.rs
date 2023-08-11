// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub struct IterMut<'a, T> {
    items: &'a mut [T],
    segment_size: usize,
}

impl<'a, T> IterMut<'a, T> {
    #[inline]
    pub fn new(items: &'a mut [T], segment_size: usize) -> Self {
        Self {
            items,
            segment_size,
        }
    }

    #[inline]
    pub fn once(items: &'a mut [T]) -> Self {
        let segment_size = items.len();
        Self::new(items, segment_size)
    }

    #[inline]
    pub fn empty() -> Self {
        Self::new(&mut [], 0)
    }
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut [T];

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let len = self.items.len().min(self.segment_size as _);

        if len == 0 {
            return None;
        }

        let (head, tail) = self.items.split_at_mut(len);

        // extend the lifetime of the slices
        let head = unsafe { core::slice::from_raw_parts_mut(head.as_mut_ptr(), head.len()) };
        let tail = unsafe { core::slice::from_raw_parts_mut(tail.as_mut_ptr(), tail.len()) };

        self.items = tail;

        Some(head)
    }
}
