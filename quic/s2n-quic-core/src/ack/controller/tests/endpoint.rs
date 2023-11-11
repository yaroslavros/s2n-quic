// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::{generator::gen_ack_settings, Packet, TestEnvironment};
use crate::{
    ack, connection, endpoint, event,
    event::{testing::Publisher, IntoEvent as _},
    frame::{ack_elicitation::AckElicitation, Ack, Frame, Ping},
    inet::DatagramInfo,
    packet::{number::PacketNumberSpace, processed::Outcome as ProcessedPacket},
    path,
    path::{path_event, testing::helper_path_server},
    time::{timer::Provider as _, Timestamp},
    transmission::{interest::Provider, Provider as _, Writer as _},
};
use bolero::generator::*;

pub use endpoint::Type;

#[derive(Clone, Debug, TypeGenerator)]
pub struct Endpoint {
    #[generator(constant(TestEnvironment::new()))]
    pub env: TestEnvironment,

    #[generator(gen_ack_settings().map_gen(new_ack_controller))]
    pub ack_controller: ack::Controller,
}

fn new_ack_controller(ack_settings: ack::Settings) -> ack::Controller {
    ack::Controller::new(PacketNumberSpace::ApplicationData, ack_settings)
}

impl Endpoint {
    pub fn new(ack_settings: ack::Settings) -> Self {
        Self {
            env: TestEnvironment::new(),
            ack_controller: new_ack_controller(ack_settings),
        }
    }

    pub fn init(&mut self, now: Timestamp, endpoint_type: endpoint::Type) {
        self.env.current_time = now;
        self.env.local_endpoint_type = endpoint_type;
    }

    pub fn recv(&mut self, packet: Packet) {
        self.env.current_time = packet.time;

        self.ack_controller.on_timeout(self.env.current_time);

        let datagram = DatagramInfo {
            ecn: packet.ecn,
            payload_len: 1200,
            timestamp: self.env.current_time,
            destination_connection_id: connection::LocalId::TEST_ID,
            destination_connection_id_classification: connection::id::Classification::Local,
            source_connection_id: None,
        };

        if let Some(ack) = packet.ack {
            for ack_range in ack.ack_ranges {
                self.ack_controller
                    .on_packet_ack(datagram.timestamp, &ack_range);
            }
        }

        let packet = ProcessedPacket {
            ack_elicitation: packet.ack_elicitation,
            datagram: &datagram,
            packet_number: packet.packet_number,
            path_challenge_on_active_path: false,
            frames: 1,
            path_validation_probing: Default::default(),
            bytes_progressed: 0,
        };

        let path = helper_path_server();
        let path_id = path::Id::test_id();
        self.ack_controller.on_processed_packet(
            &packet,
            path_event!(path, path_id),
            &mut Publisher::no_snapshot(),
        );
    }

    pub fn send(&mut self, now: Timestamp) -> Option<Packet> {
        self.env.current_time = now;
        self.ack_controller.on_timeout(now);
        self.transmit(AckElicitation::Eliciting)
    }

    pub fn tick(&mut self, now: Timestamp) -> Option<Packet> {
        self.env.current_time = now;
        self.ack_controller.on_timeout(now);

        if !self.ack_controller.has_transmission_interest() {
            return None;
        }

        self.transmit(AckElicitation::NonEliciting)
    }

    pub fn timers(&self) -> impl Iterator<Item = Timestamp> {
        self.ack_controller.next_expiration().into_iter()
    }

    fn transmit(&mut self, ack_elicitation: AckElicitation) -> Option<Packet> {
        let mut context = self.env.context();
        let did_send_ack = self.ack_controller.on_transmit(&mut context);

        if ack_elicitation.is_ack_eliciting() {
            context.write_frame(&Ping);
        }

        if did_send_ack {
            self.ack_controller.on_transmit_complete(&mut context);
        }

        let ack_elicitation = context.ack_elicitation();
        let packet_number = context.packet_number();

        if self.env.sent_frames.is_empty() {
            return None;
        }

        let mut packet = Packet {
            packet_number,
            ack_elicitation,
            ecn: Default::default(),
            time: self.env.current_time,
            ack: None,
        };

        while let Some(mut frame) = self.env.sent_frames.pop_front() {
            if let Frame::Ack(ack) = frame.as_frame() {
                packet.ack = Some(Ack {
                    ack_delay: ack.ack_delay,
                    ecn_counts: ack.ecn_counts,
                    ack_ranges: ack
                        .ack_ranges()
                        .map(|ack_range| {
                            let (start, end) = ack_range.into_inner();

                            let pn_space = PacketNumberSpace::ApplicationData;
                            let start = pn_space.new_packet_number(start);
                            let end = pn_space.new_packet_number(end);

                            start..=end
                        })
                        .collect(),
                });
            }
        }

        self.env.sent_frames.flush();

        Some(packet)
    }

    pub fn done(&mut self) {
        assert!(
            !self.ack_controller.has_transmission_interest(),
            "ack manager should be in a stable state"
        );
    }
}
