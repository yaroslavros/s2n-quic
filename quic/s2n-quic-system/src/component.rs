use crate::prelude::*;
use s2n_quic_core::application;

pub fn setup<C: Config>(world: &mut World, config: &mut C) {
    crypto::setup(world, config);
}

pub mod connection {
    use super::*;
    use s2n_quic_core::connection as c;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Connection;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct LocalId(pub c::LocalId);

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct InitialId(pub c::InitialId);
}

pub mod crypto {
    use super::*;
    use s2n_quic_core::crypto as c;

    #[derive(Component)]
    pub struct Initial<K: 'static + c::InitialKey>(pub K);

    #[derive(Component)]
    pub struct Handshake<K: 'static + c::HandshakeKey>(pub K);

    #[derive(Component)]
    pub struct OneRtt<K: 'static + c::OneRttKey>(pub K);

    pub fn setup<C: Config>(world: &mut World, _config: &mut C) {
        world.register::<Initial<<<C::TLSEndpoint as c::tls::Endpoint>::Session as c::CryptoSuite>::InitialKey>>();
        world.register::<Handshake<
            <<C::TLSEndpoint as c::tls::Endpoint>::Session as c::CryptoSuite>::HandshakeKey,
        >>();
        world.register::<OneRtt<
            <<C::TLSEndpoint as c::tls::Endpoint>::Session as c::CryptoSuite>::OneRttKey,
        >>();
    }
}

#[derive(Clone, Component)]
pub struct ServerName(pub application::ServerName);

pub mod inet {
    use super::*;
    use s2n_quic_core::inet::IpAddress;

    #[derive(Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct RemoteAddress(pub IpAddress);

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct RemotePort(pub u16);

    #[derive(Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct LocalAddress(pub IpAddress);

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct LocalPort(pub u16);
}

pub mod time {
    use super::*;
    use s2n_quic_core::time;

    #[derive(Clone, Default, PartialEq, Eq, Component)]
    pub struct Timer(pub time::Timer);
}

pub mod packet {
    use super::*;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Packet;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Rx;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Tx;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Initial;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Handshake;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Application;

    #[derive(Clone, Copy, Default, PartialEq, Eq, Ord, PartialOrd, Hash, Component)]
    pub struct Unknown;
}
