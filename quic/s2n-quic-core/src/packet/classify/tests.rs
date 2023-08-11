// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::connection::id;
use bolero::check;
use s2n_codec::{DecoderBufferMut, DecoderError};

fn run(buffer: &mut [u8]) -> Result<(), DecoderError> {
    let decoder = DecoderBufferMut::new(buffer);

    let (dcid_len, decoder) = decoder.decode::<u8>()?;
    let dcid_len = dcid_len as usize;
    let (segment_size, decoder) = decoder.decode::<u16>()?;
    let payload = decoder.into_less_safe_slice();
    let payload = crate::slice::segments::IterMut::new(payload, segment_size as _);

    let socket_addr = Default::default();

    let connection_info = id::ConnectionInfo::new(&socket_addr);

    super::datagram(&dcid_len, &connection_info, payload, &mut ());

    Ok(())
}

#[test]
fn fuzz_test() {
    check!().for_each(|bytes| {
        let bytes = unsafe {
            // SAFETY: we don't actually mutate any of the bytes in the test
            core::slice::from_raw_parts_mut(bytes.as_ptr() as *mut _, bytes.len())
        };
        let _ = run(bytes);
    })
}
