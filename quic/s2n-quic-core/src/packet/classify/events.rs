// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::Cursor;
use crate::{
    connection::id::LocalId,
    event::{self, EndpointPublisher as _},
    packet::ProtectedPacket,
};
use s2n_codec::DecoderError;

pub trait Events {
    fn on_packet(&mut self, cursor: &Cursor, dcid: &LocalId, packet: ProtectedPacket);

    fn on_invalid_destination_connection_id(&mut self, cursor: &Cursor, packet: ProtectedPacket);

    fn on_mismatched_destination_connection_id(
        &mut self,
        cursor: &Cursor,
        expected_dcid: &LocalId,
        packet: ProtectedPacket,
    );

    fn on_unparseable_packet(&mut self, cursor: &Cursor, payload: &mut [u8], err: DecoderError);
}

#[cfg_attr(fuzz, inline(never))]
#[cfg_attr(not(fuzz), inline(always))]
fn noop() {
    // noop
}

impl Events for () {
    #[inline(always)]
    fn on_packet(&mut self, _cursor: &Cursor, _dcid: &LocalId, _packet: ProtectedPacket) {
        noop()
    }

    #[inline(always)]
    fn on_invalid_destination_connection_id(&mut self, _cursor: &Cursor, _packet: ProtectedPacket) {
        noop()
    }

    #[inline(always)]
    fn on_mismatched_destination_connection_id(
        &mut self,
        _cursor: &Cursor,
        _expected_dcid: &LocalId,
        _packet: ProtectedPacket,
    ) {
        noop()
    }

    #[inline(always)]
    fn on_unparseable_packet(&mut self, _cursor: &Cursor, _payload: &mut [u8], _err: DecoderError) {
        noop()
    }
}

impl<'a, Sub: event::Subscriber> Events for event::EndpointPublisherSubscriber<'a, Sub> {
    #[inline(always)]
    fn on_packet(&mut self, _cursor: &Cursor, _dcid: &LocalId, _packet: ProtectedPacket) {
        noop()
    }

    #[inline]
    fn on_invalid_destination_connection_id(&mut self, cursor: &Cursor, _packet: ProtectedPacket) {
        let len = cursor.len.try_into().ok().unwrap_or(u16::MAX);
        self.on_endpoint_datagram_dropped(event::builder::EndpointDatagramDropped {
            len,
            reason: event::builder::DatagramDropReason::InvalidDestinationConnectionId,
        });
    }

    #[inline]
    fn on_mismatched_destination_connection_id(
        &mut self,
        cursor: &Cursor,
        _expected_dcid: &LocalId,
        _packet: ProtectedPacket,
    ) {
        let len = cursor.len.try_into().ok().unwrap_or(u16::MAX);
        self.on_endpoint_datagram_dropped(event::builder::EndpointDatagramDropped {
            len,
            reason: event::builder::DatagramDropReason::InvalidDestinationConnectionId,
        });
    }

    #[inline]
    fn on_unparseable_packet(&mut self, _cursor: &Cursor, payload: &mut [u8], _err: DecoderError) {
        let len = payload.len().try_into().ok().unwrap_or(u16::MAX);
        self.on_endpoint_datagram_dropped(event::builder::EndpointDatagramDropped {
            len,
            reason: event::builder::DatagramDropReason::DecodingFailed,
        });
    }
}
