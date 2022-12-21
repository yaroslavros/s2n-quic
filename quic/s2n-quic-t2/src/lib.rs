pub mod buffer;
pub mod connection;
pub mod decrypt;
pub mod inbound;
pub mod prelude;
pub mod temporary;

#[test]
fn testing() {
    use prelude::*;

    let mut world = World::new();

    connection::setup(&mut world);

    type Format = s2n_quic_core::connection::id::testing::Format;

    world.insert_resource(connection::Format(Format::default()));

    let socket_count = 1u32;
    let socket_buffer = 1500u32 * 1024;

    let mut bump = buffer::Bump::new(socket_buffer * socket_count);

    for _ in 0..socket_count {
        world.spawn(socket(4433, bump.alloc(socket_buffer as _).unwrap()));
    }

    world.insert_resource(bump);

    let mut schedule = Schedule::default();

    schedule.add_stage(
        inbound::RecvLabel,
        SystemStage::parallel().with_system(inbound::recv),
    );

    schedule.add_stage(
        inbound::ParseLabel,
        SystemStage::parallel().with_system(inbound::parse::<Format>),
    );

    #[derive(Debug, StageLabel)]
    struct Finish;

    // clean anything up
    schedule.add_stage(
        Finish,
        SystemStage::parallel()
            .with_system(temporary::cleanup)
            .with_system(buffer::tick),
    );

    loop {
        schedule.run(&mut world);
    }
}

fn socket(port: u16, heap: buffer::Heap) -> (inbound::Socket, buffer::Heap, inbound::Local) {
    // TODO reuse port/addr
    let socket = std::net::UdpSocket::bind(("0.0.0.0", port)).unwrap();

    socket.set_nonblocking(true).unwrap();

    let local = socket.local_addr().unwrap();
    let socket = inbound::Socket(socket);
    let local = inbound::Local(local.into());
    (socket, heap, local)
}
