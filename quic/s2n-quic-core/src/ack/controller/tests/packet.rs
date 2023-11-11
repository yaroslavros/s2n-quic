// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    frame::{ack_elicitation::AckElicitation, Ack},
    inet::ExplicitCongestionNotification,
    packet::number::PacketNumber,
    time::Timestamp,
};
use alloc::collections::VecDeque;
use core::ops::RangeInclusive;

#[derive(Clone, Debug)]
pub struct Packet {
    pub packet_number: PacketNumber,
    pub ack_elicitation: AckElicitation,
    pub ecn: ExplicitCongestionNotification,
    pub time: Timestamp,
    pub ack: Option<Ack<VecDeque<RangeInclusive<PacketNumber>>>>,
}
