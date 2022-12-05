use crate::prelude::*;
use s2n_quic_transport::endpoint::Config;

mod component;
mod dispatch;
mod prelude;
mod resource;
mod system;
mod world;

pub struct Endpoint {
    world: World,
    // TODO what should these lifetimes be?
    dispatchers: Dispatchers<'static, 'static>,
}

impl Endpoint {
    pub fn new<C: Config>(mut config: C) -> Self {
        let mut world = World::new();

        component::setup(&mut world, &mut config);
        resource::setup(&mut world, &mut config);

        let mut dispatchers = DispatcherBuilders::default();

        system::setup(&mut dispatchers, &mut config);

        let dispatchers = dispatchers.build();

        // TODO impl Sync
        // world.insert(config);

        Self { world, dispatchers }
    }
}
