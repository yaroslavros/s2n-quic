// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    frame::{ack_elicitation::AckElicitation, path_validation, FrameTrait},
    inet::{DatagramInfo, ExplicitCongestionNotification},
    packet::number::PacketNumber,
    time::Timestamp,
};

/// Tracks information about a packet that has been processed
#[derive(Clone, Copy, Debug)]
pub struct Outcome<'a> {
    pub packet_number: PacketNumber,
    pub datagram: &'a DatagramInfo,
    pub ack_elicitation: AckElicitation,
    pub path_challenge_on_active_path: bool,
    pub frames: usize,
    pub path_validation_probing: path_validation::Probe,
    pub bytes_progressed: usize,
}

impl<'a> Outcome<'a> {
    /// Creates a processed packet tracker
    #[inline]
    pub fn new(packet_number: PacketNumber, datagram: &'a DatagramInfo) -> Self {
        Self {
            packet_number,
            datagram,
            ack_elicitation: AckElicitation::default(),
            path_challenge_on_active_path: false,
            frames: 0,
            path_validation_probing: path_validation::Probe::default(),
            bytes_progressed: 0,
        }
    }

    /// Records information about a processed frame
    #[inline]
    pub fn on_processed_frame<F: FrameTrait>(&mut self, frame: &F) {
        self.ack_elicitation |= frame.ack_elicitation();
        self.frames += 1;
        self.path_validation_probing |= frame.path_validation();
    }
}

impl<'a> crate::ack::controller::ReceivedPacket for Outcome<'a> {
    #[inline]
    fn packet_number(&self) -> PacketNumber {
        self.packet_number
    }

    #[inline]
    fn ecn(&self) -> ExplicitCongestionNotification {
        self.datagram.ecn
    }

    #[inline]
    fn path_challenge_on_active_path(&self) -> bool {
        self.path_challenge_on_active_path
    }

    #[inline]
    fn is_ack_eliciting(&self) -> bool {
        self.ack_elicitation.is_ack_eliciting()
    }

    #[inline]
    fn timestamp(&self) -> Timestamp {
        self.datagram.timestamp
    }
}
