// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    connection::id::{ConnectionInfo, LocalId, Validator},
    packet::ProtectedPacket,
};
use s2n_codec::DecoderBufferMut;

#[cfg(test)]
mod tests;

pub mod events;
pub use events::Events;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    /// The segment index into the payload
    pub segment: u32,
    /// The byte offset into the whole payload where the packet starts
    pub offset: u32,
    /// The index of the QUIC packet in the current segment
    pub index: u32,
    /// The total length of the QUIC packet, including the header
    pub len: u32,
}

#[inline]
pub fn datagram<'a, V: Validator, E: Events, P: Iterator<Item = &'a mut [u8]>>(
    validator: &V,
    connection_info: &ConnectionInfo,
    payload: P,
    events: &mut E,
) {
    let mut cursor = Cursor {
        segment: 0,
        offset: 0,
        index: 0,
        len: 0,
    };

    for segment in payload {
        let len = segment.len();
        datagram_segment(validator, connection_info, cursor, segment, events);
        cursor.segment += 1;
        cursor.offset += len as u32;
    }
}

#[inline]
fn datagram_segment<V: Validator, E: Events>(
    validator: &V,
    connection_info: &ConnectionInfo,
    mut cursor: Cursor,
    payload: &mut [u8],
    events: &mut E,
) {
    // keep track of the quic packet index
    let mut destination_connection_id = None;

    let mut buffer = DecoderBufferMut::new(payload);

    while !buffer.is_empty() {
        // take a snapshot of the current buffer length
        let buffer_len = buffer.len();

        // try decoding a packet from the current buffer
        match ProtectedPacket::decode(buffer, connection_info, validator) {
            Ok((packet, remaining)) => {
                // compute how many bytes we consumed for the current packet
                let packet_len = buffer_len - remaining.len();
                cursor.len = packet_len as _;

                // assign the buffer to the remaining bytes
                buffer = remaining;

                //= https://www.rfc-editor.org/rfc/rfc9000#section-12.2
                //# Every QUIC packet that is coalesced into a single UDP datagram is
                //# separate and complete.  The receiver of coalesced QUIC packets MUST
                //# individually process each QUIC packet and separately acknowledge
                //# them, as if they were received as the payload of different UDP
                //# datagrams.  For example, if decryption fails (because the keys are
                //# not available or for any other reason), the receiver MAY either
                //# discard or buffer the packet for later processing and MUST attempt to
                //# process the remaining packets.
                quic_packet(cursor, &mut destination_connection_id, packet, events);

                cursor.index += 1;
                cursor.offset += packet_len as u32;
            }
            Err(err) => {
                // We choose to discard the rest of the datagram on parsing errors since it
                // would be difficult to recover from an invalid packet.
                events.on_unparseable_packet(&cursor, payload, err);

                break;
            }
        }
    }
}

#[inline]
fn quic_packet<E: Events>(
    cursor: Cursor,
    dcid: &mut Option<LocalId>,
    packet: ProtectedPacket,
    events: &mut E,
) {
    let actual_dcid = packet.destination_connection_id();

    let dcid = if let Some(expected_dcid) = dcid.as_ref() {
        //= https://www.rfc-editor.org/rfc/rfc9000#section-12.2
        //# Senders MUST NOT coalesce QUIC packets
        //# with different connection IDs into a single UDP datagram.  Receivers
        //# SHOULD ignore any subsequent packets with a different Destination
        //# Connection ID than the first packet in the datagram.
        if expected_dcid.as_bytes() != packet.destination_connection_id() {
            events.on_mismatched_destination_connection_id(&cursor, expected_dcid, packet);
            return;
        }

        expected_dcid
    } else {
        match LocalId::try_from_bytes(actual_dcid) {
            Some(cid) => {
                *dcid = Some(cid);
                dcid.as_ref().unwrap()
            }
            None => {
                events.on_invalid_destination_connection_id(&cursor, packet);
                return;
            }
        }
    };

    events.on_packet(&cursor, dcid, packet);
}
