// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use super::{
    ack_eliciting_transmission::Transmission as AckElicitingTransmission, transmission_state, *,
};
use crate::{
    ack::{self, Controller},
    connection,
    event::testing::Publisher,
    frame::{ack_elicitation::AckElicitation, ping, Frame},
    inet::{DatagramInfo, ExplicitCongestionNotification},
    packet::{
        number::{PacketNumber, PacketNumberSpace},
        processed::Outcome as ProcessedPacket,
    },
    path,
    path::{path_event, testing::helper_path_server},
    time::{clock::testing as time, Clock, NoopClock},
    transmission::writer::testing::{OutgoingFrameBuffer, Writer as MockWriteContext},
    varint::VarInt,
};
use core::{
    iter::{empty, once},
    time::Duration,
};
use insta::assert_debug_snapshot;

/// Generates AckElicitingTransmissions from increasing packet numbers
pub fn transmissions_iter() -> impl Iterator<Item = AckElicitingTransmission> {
    packet_numbers_iter().map(|pn| AckElicitingTransmission {
        sent_in_packet: pn,
        largest_received_packet_number_acked: pn,
    })
}

/// Generates increasing packet numbers
pub fn packet_numbers_iter() -> impl Iterator<Item = PacketNumber> {
    Iterator::map(0u32.., |pn| {
        PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u32(pn))
    })
}

pub mod application;
pub mod endpoint;
pub mod environment;
pub mod generator;
pub mod network;
pub mod network_event;
pub mod network_interface;
pub mod packet;
pub mod report;
pub mod simulation;

pub use application::*;
pub use endpoint::*;
pub use environment::*;
pub use network::*;
pub use network_event::*;
pub use network_interface::*;
pub use packet::*;
pub use report::*;
pub use simulation::*;

#[test]
fn activate() {
    // Setup:
    let mut manager = Controller::new(PacketNumberSpace::ApplicationData, ack::Settings::default());

    let pn = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(1));
    let datagram = DatagramInfo {
        ecn: Default::default(),
        payload_len: 1200,
        timestamp: NoopClock {}.get_time(),
        destination_connection_id: connection::LocalId::TEST_ID,
        destination_connection_id_classification: connection::id::Classification::Local,
        source_connection_id: None,
    };
    let mut processed_packet = ProcessedPacket::new(pn, &datagram);
    processed_packet.path_challenge_on_active_path = true;
    processed_packet.ack_elicitation = AckElicitation::Eliciting;

    assert!(!manager.transmission_state.is_active());

    // Trigger:
    let path = helper_path_server();
    let path_id = path::Id::test_id();
    manager.on_processed_packet(
        &processed_packet,
        path_event!(path, path_id),
        &mut Publisher::snapshot(),
    );

    // Expectation:
    assert!(manager.transmission_state.is_active());
}

#[test]
fn ecn_counts() {
    // Setup:
    let mut manager = Controller::new(PacketNumberSpace::ApplicationData, ack::Settings::default());

    assert_eq!(0, manager.ecn_counts.ect_0_count.as_u64());
    assert_eq!(0, manager.ecn_counts.ect_1_count.as_u64());
    assert_eq!(0, manager.ecn_counts.ce_count.as_u64());

    // Process a packet in an Ect0 datagram
    let pn = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(1));
    let datagram = helper_datagram_info(ExplicitCongestionNotification::Ect0);
    let path = helper_path_server();
    let path_id = path::Id::test_id();
    let mut publisher = Publisher::snapshot();
    manager.on_processed_packet(
        &ProcessedPacket::new(pn, &datagram),
        path_event!(path, path_id),
        &mut publisher,
    );

    assert_eq!(1, manager.ecn_counts.ect_0_count.as_u64());
    assert_eq!(0, manager.ecn_counts.ect_1_count.as_u64());
    assert_eq!(0, manager.ecn_counts.ce_count.as_u64());

    // Process a couple packets in an Ect1 datagram
    let pn1 = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(2));
    let pn2 = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(3));
    let datagram = helper_datagram_info(ExplicitCongestionNotification::Ect1);

    manager.on_processed_packet(
        &ProcessedPacket::new(pn1, &datagram),
        path_event!(path, path_id),
        &mut publisher,
    );
    manager.on_processed_packet(
        &ProcessedPacket::new(pn2, &datagram),
        path_event!(path, path_id),
        &mut publisher,
    );

    assert_eq!(1, manager.ecn_counts.ect_0_count.as_u64());
    assert_eq!(2, manager.ecn_counts.ect_1_count.as_u64());
    assert_eq!(0, manager.ecn_counts.ce_count.as_u64());

    // Process a packet in an Ce datagram
    let pn = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(4));
    let datagram = helper_datagram_info(ExplicitCongestionNotification::Ce);
    manager.on_processed_packet(
        &ProcessedPacket::new(pn, &datagram),
        path_event!(path, path_id),
        &mut publisher,
    );

    assert_eq!(1, manager.ecn_counts.ect_0_count.as_u64());
    assert_eq!(2, manager.ecn_counts.ect_1_count.as_u64());
    assert_eq!(1, manager.ecn_counts.ce_count.as_u64());

    // Process a packet in a NotEct datagram
    let pn = PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(5));
    let datagram = helper_datagram_info(ExplicitCongestionNotification::NotEct);
    manager.on_processed_packet(
        &ProcessedPacket::new(pn, &datagram),
        path_event!(path, path_id),
        &mut publisher,
    );

    assert_eq!(1, manager.ecn_counts.ect_0_count.as_u64());
    assert_eq!(2, manager.ecn_counts.ect_1_count.as_u64());
    assert_eq!(1, manager.ecn_counts.ce_count.as_u64());
}

/// Helper function to construct `DatagramInfo` with the given `ExplicitCongestionNotification`
fn helper_datagram_info(ecn: ExplicitCongestionNotification) -> DatagramInfo {
    DatagramInfo {
        ecn,
        payload_len: 1200,
        timestamp: NoopClock {}.get_time(),
        destination_connection_id: connection::LocalId::TEST_ID,
        destination_connection_id_classification: connection::id::Classification::Local,
        source_connection_id: None,
    }
}

#[test]
fn on_transmit_complete_transmission_constrained() {
    let mut manager = Controller::new(PacketNumberSpace::ApplicationData, ack::Settings::default());
    let mut frame_buffer = OutgoingFrameBuffer::new();
    let mut write_context = MockWriteContext::new(
        time::now(),
        &mut frame_buffer,
        transmission::Constraint::None,
        transmission::Mode::Normal,
        endpoint::Type::Server,
    );

    manager.ack_ranges = ack::Ranges::default();
    assert!(manager
        .ack_ranges
        .insert_packet_number(
            PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(1)),
        )
        .is_ok());
    manager.transmission_state = transmission_state::State::Active { retransmissions: 0 };
    manager.transmissions_since_elicitation =
        Counter::new(ack::Settings::EARLY.ack_elicitation_interval);

    manager.on_transmit_complete(&mut write_context);

    assert_eq!(
        write_context
            .frame_buffer
            .pop_front()
            .expect("Frame is written")
            .as_frame(),
        Frame::Ping(ping::Ping),
        "Ping should be written when transmission is not constrained"
    );

    manager.transmission_state = transmission_state::State::Active { retransmissions: 0 };
    manager.transmissions_since_elicitation =
        Counter::new(ack::Settings::EARLY.ack_elicitation_interval);
    write_context.frame_buffer.clear();
    write_context.transmission_constraint = transmission::Constraint::CongestionLimited;

    manager.on_transmit_complete(&mut write_context);
    assert!(
        write_context.frame_buffer.is_empty(),
        "Ping should not be written when CongestionLimited"
    );

    manager.transmission_state = transmission_state::State::Active { retransmissions: 0 };
    manager.transmissions_since_elicitation =
        Counter::new(ack::Settings::EARLY.ack_elicitation_interval);
    write_context.frame_buffer.clear();
    write_context.transmission_constraint = transmission::Constraint::RetransmissionOnly;

    manager.on_transmit_complete(&mut write_context);
    assert_eq!(
        write_context
            .frame_buffer
            .pop_front()
            .expect("Frame is written")
            .as_frame(),
        Frame::Ping(ping::Ping),
        "Ping should be written when transmission is retransmission only"
    );
}

#[test]
fn on_transmit_complete_many_transmissions_since_elicitation() {
    let mut manager = Controller::new(PacketNumberSpace::ApplicationData, ack::Settings::default());
    let mut frame_buffer = OutgoingFrameBuffer::new();
    let mut write_context = MockWriteContext::new(
        time::now(),
        &mut frame_buffer,
        transmission::Constraint::None,
        transmission::Mode::Normal,
        endpoint::Type::Server,
    );
    write_context.transmission_constraint = transmission::Constraint::CongestionLimited;

    manager.ack_ranges = ack::Ranges::default();
    assert!(manager
        .ack_ranges
        .insert_packet_number(
            PacketNumberSpace::ApplicationData.new_packet_number(VarInt::from_u8(1)),
        )
        .is_ok());
    manager.transmission_state = transmission_state::State::Active { retransmissions: 0 };
    manager.transmissions_since_elicitation = Counter::new(u8::max_value());

    manager.on_transmit_complete(&mut write_context);

    assert_eq!(
        manager.transmissions_since_elicitation,
        Counter::new(u8::max_value())
    );
}

#[test]
#[cfg(target_pointer_width = "64")]
fn size_of_snapshots() {
    use core::mem::size_of;
    use insta::assert_debug_snapshot;

    assert_debug_snapshot!("AckManager", size_of::<Controller>());
}

#[test]
fn client_sending_test() {
    assert_debug_snapshot!(
        "client_sending_test",
        Simulation {
            network: Network {
                client: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
                server: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    empty(),
                )
                .into(),
            },
            // pass all packets unchanged
            events: empty().collect(),
            delay: Duration::from_millis(0),
        }
        .run()
    );
}

#[test]
fn delayed_client_sending_test() {
    assert_debug_snapshot!(
        "delayed_client_sending_test",
        Simulation {
            network: Network {
                client: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
                server: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    empty(),
                )
                .into(),
            },
            // pass all packets unchanged
            events: empty().collect(),
            // delay sending each packet by 100ms
            delay: Duration::from_millis(100),
        }
        .run()
    );
}

#[test]
fn high_latency_test() {
    assert_debug_snapshot!(
        "high_latency_test",
        Simulation {
            network: Network {
                client: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
                server: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(100),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
            },
            // pass all packets unchanged
            events: empty().collect(),
            // delay sending each packet by 1s
            delay: Duration::from_millis(1000),
        }
        .run()
    );
}

#[test]
fn lossy_network_test() {
    assert_debug_snapshot!(
        "lossy_network_test",
        Simulation {
            network: Network {
                client: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(25),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
                server: Application::new(
                    Endpoint::new(ack::Settings {
                        max_ack_delay: Duration::from_millis(100),
                        ack_delay_exponent: 1,
                        ..Default::default()
                    }),
                    [Duration::from_millis(5)].iter().cycle().take(100).cloned(),
                )
                .into(),
            },
            // drop every 5th packet
            events: once(NetworkEvent::Pass)
                .cycle()
                .take(4)
                .chain(once(NetworkEvent::Drop))
                .collect(),
            // delay sending each packet by 100ms
            delay: Duration::from_millis(0),
        }
        .run()
    );
}

#[test]
fn simulation_harness_test() {
    use core::time::Duration;

    let client_transmissions = 100;
    let server_transmissions = 10;

    let mut simulation = Simulation {
        network: Network {
            client: Application::new(
                Endpoint::new(ack::Settings {
                    max_ack_delay: Duration::from_millis(25),
                    ack_delay_exponent: 1,
                    ..Default::default()
                }),
                // send an ack-eliciting packet every 5ms, 100 times
                [Duration::from_millis(5)]
                    .iter()
                    .cycle()
                    .take(client_transmissions)
                    .cloned(),
            )
            .into(),
            server: Application::new(
                Endpoint::new(ack::Settings {
                    max_ack_delay: Duration::from_millis(25),
                    ack_delay_exponent: 1,
                    ..Default::default()
                }),
                // send an ack-eliciting packet every 800ms, 10 times
                [Duration::from_millis(800)]
                    .iter()
                    .cycle()
                    .take(server_transmissions)
                    .cloned(),
            )
            .into(),
        },
        // pass all packets unchanged
        events: [NetworkEvent::Pass].iter().cloned().collect(),
        // delay sending each packet by 100ms
        delay: Duration::from_millis(100),
    };

    let report = simulation.run();

    assert!(report.client.ack_eliciting_transmissions >= client_transmissions);
    assert!(report.client.dropped_transmissions == 0);

    assert!(report.server.ack_eliciting_transmissions >= server_transmissions);
    assert!(report.server.dropped_transmissions == 0);
}

/// Ack Manager Simulation
///
/// This target will generate and execute fuzz-guided simulation scenarios.
///
/// The following assertions are made:
///
/// * The program doesn't crash
///
/// * Two AckManagers talking to each other in various configurations
///   and network scenarios will always terminate (not endlessly ACK each other)
///
/// Additional checks may be implemented at some point to expand guarantees
#[test]
fn simulation_test() {
    bolero::check!()
        .with_type::<Simulation>()
        .cloned()
        .for_each(|mut simulation| {
            let _report = simulation.run();

            // TODO make assertions about report
        });
}
