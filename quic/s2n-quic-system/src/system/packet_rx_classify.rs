use crate::{
    component::{
        connection::{InitialId, LocalId},
        packet::{Packet, Rx, Unknown},
    },
    prelude::*,
    resource::connection_id::{Initial, Local},
};

struct Sys;

pub static NAME: &str = module_path!();

impl<'a> System<'a> for Sys {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, InitialId>,
        ReadStorage<'a, Rx>,
        WriteStorage<'a, Unknown>,
        Read<'a, Initial>,
    );

    fn run(&mut self, (entities, id, rx, mut unknown, ids): Self::SystemData) {
        for (entity, conn_id, _) in (&entities, &id, &rx).join() {
            if let Some(connection) = ids.get(&conn_id.0) {
                // TODO associate with connection
                let _ = connection;
            } else {
                let _ = unknown.insert(entity, Unknown);
            }
        }
    }
}

pub fn setup<C: Config>(dispatch: &mut DispatcherBuilders, _config: &mut C) {
    dispatch.rx.add(Sys, NAME, &[]);
}
