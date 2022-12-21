use crate::{
    buffer::{Bump, Heap, Range},
    connection::{InitialIdMap, LocalIdMap},
    prelude::*,
    temporary,
};
use s2n_codec::{CheckedRange, DecoderBufferMut};
use s2n_quic_core::{
    connection::{self, id},
    inet::{ExplicitCongestionNotification, SocketAddress},
    packet::{self, number::ProtectedPacketNumber, ProtectedPacket},
};
use std::net::UdpSocket;

#[derive(Debug, Component)]
pub struct Socket(pub UdpSocket);

#[derive(Debug, Bundle)]
struct DatagramBundle {
    local: Local,
    remote: Remote,
    payload: Payload,
    ecn: Ecn,
    temporary: temporary::Marker,
}

#[derive(Clone, Copy, Debug, Component)]
pub struct Datagram(pub Entity);

#[derive(Debug, Component)]
pub struct Payload(pub Range);

#[derive(Clone, Copy, Debug, Component)]
pub struct Local(pub SocketAddress);

#[derive(Debug, Component)]
pub struct Remote(pub SocketAddress);

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct Ecn(pub ExplicitCongestionNotification);

#[derive(Debug, Component)]
pub struct ProtectedInitial(
    packet::initial::Initial<
        CheckedRange,
        CheckedRange,
        CheckedRange,
        ProtectedPacketNumber,
        PayloadRange,
    >,
);

#[derive(Debug, Component)]
pub struct ProtectedHandshake(
    packet::handshake::Handshake<CheckedRange, CheckedRange, ProtectedPacketNumber, PayloadRange>,
);

#[derive(Debug)]
struct PayloadRange {
    header_len: usize,
    payload: Range,
}

#[derive(Clone, Copy, Debug, StageLabel)]
pub struct RecvLabel;

pub fn recv(mut sockets: Query<(&mut Socket, &mut Heap, &Local)>, commands: ParallelCommands) {
    sockets.par_for_each_mut(1, |(socket, mut heap, local)| {
        let mut heap = heap.aquire();
        commands.command_scope(|mut commands| {
            while let Some(mut range) = heap.alloc(1500) {
                let payload = heap.get(&mut range);

                match socket.0.recv_from(payload) {
                    Ok((len, remote)) => {
                        if len == 0 {
                            continue;
                        }

                        if let Some(payload) = range.split_to(len) {
                            // return what we didn't use
                            heap.dealloc(range);

                            let payload = Payload(payload);
                            let remote = Remote(remote.into());
                            commands.spawn(DatagramBundle {
                                payload,
                                local: *local,
                                remote,
                                ecn: Default::default(),
                                temporary: Default::default(),
                            });
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        //
                        break;
                    }
                    Err(err) => {
                        // TODO emit error
                        let _ = dbg!(err);
                        break;
                    }
                }
            }
        });
    })
}

/*
pub fn intercept_datagram<I: Interceptor + 'static + Send + Sync>(
    mut query: Query<(&mut Payload, &Local, &Remote)>,
    bump: Res<Bump>,
    interceptor: ResMut<Provider<I>>,
) {
    for (mut payload, local, remote) in &mut query {
        let buffer = bump.get(&mut payload.0);
        let buffer = DecoderBufferMut::new(buffer);

        // TODO
        // interceptor.0.intercept_rx_datagram();
    }
}
*/

#[derive(Clone, Copy, Debug, StageLabel)]
pub struct ParseLabel;

pub fn parse<F: id::Format + 'static + Send + Sync>(
    format: Res<crate::connection::Format<F>>,
    mut query: Query<(Entity, &mut Payload, &Remote)>,
    bump: Res<Bump>,
    local_ids: Res<LocalIdMap>,
    initial_ids: Res<InitialIdMap>,
    commands: ParallelCommands,
) {
    const BATCH_SIZE: usize = 64;

    query.par_for_each_mut(BATCH_SIZE, |(datagram, mut payload, remote)| {
        commands.command_scope(|mut commands| {
            let datagram = Datagram(datagram);
            let connection_info = id::ConnectionInfo::new(&remote.0);

            while !payload.0.is_empty() {
                parse_packet(
                    &bump,
                    &connection_info,
                    datagram,
                    &format.0,
                    &local_ids,
                    &initial_ids,
                    &mut payload.0,
                    &mut commands,
                );
            }
        });
    });
}

fn parse_packet<F: id::Validator>(
    bump: &Bump,
    connection_info: &id::ConnectionInfo,
    datagram: Datagram,
    format: &F,
    local_ids: &LocalIdMap,
    initial_ids: &InitialIdMap,
    payload: &mut Range,
    commands: &mut Commands,
) {
    let buffer = bump.get(payload);
    let buffer = DecoderBufferMut::new(buffer);

    let packet = if let Ok((packet, _remaining)) =
        ProtectedPacket::decode(buffer, connection_info, format)
    {
        packet
    } else {
        // TODO emit error event
        payload.clear();
        return;
    };

    // TODO ensure the version is supported

    let dcid = connection::LocalId::try_from_bytes(packet.destination_connection_id());

    let packet = packet
        .map_payload(|p| {
            let (header_len, buffer) = p.into_less_safe_payload();
            (header_len, buffer.len())
        })
        .map_payload(|(header_len, buffer_len)| {
            let payload = payload.split_to(buffer_len).unwrap();
            PayloadRange {
                payload,
                header_len,
            }
        });

    let dcid = if let Some(dcid) = dcid {
        dcid
    } else {
        return;
    };

    macro_rules! process_non_initial_packet {
        ($ty:ident, $packet:expr) => {{
            let packet = $packet;
            if let Ok(id) = connection::LocalId::try_from(packet.destination_connection_id()) {
                let packet = map_payload!(packet);
                let packet = $ty(packet);

                if let Some(connection) = local_ids.get(&id) {
                    dbg!(&packet);
                    commands.spawn((packet, datagram, connection, temporary::Marker));
                } else {
                    // TODO spawn unknown connection
                }
            } else {
                // TODO spawn invalid connection id
            }
        }};
    }

    match packet {
        ProtectedPacket::Initial(packet) => {
            if let Ok(id) = connection::InitialId::try_from(packet.destination_connection_id()) {}

            let packet = map_payload!(packet);
            let packet = ProtectedInitial(packet);
            dbg!(&packet);
            commands.spawn((packet, datagram, temporary::Marker));
        }
        ProtectedPacket::Handshake(packet) => {
            process_non_initial_packet!(ProtectedHandshake, packet);
        }
        _ => {
            // TODO
            payload.clear();
        }
    }
}
