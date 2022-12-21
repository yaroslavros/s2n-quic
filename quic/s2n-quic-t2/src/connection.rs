use crate::prelude::*;
use s2n_quic_core::connection;
use std::collections::{hash_map::Entry, HashMap};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Component)]
pub struct Connection(Entity);

#[derive(Debug, Resource)]
pub struct Format<P: 'static + Send + Sync + connection::id::Format>(pub P);

pub fn setup(world: &mut World) {
    world.insert_resource(LocalIdMap::default());
    world.insert_resource(InitialIdMap::default());
}

#[derive(Default, Debug, Resource)]
pub struct LocalIdMap {
    mapping: HashMap<connection::LocalId, Connection>,
}

impl LocalIdMap {
    pub fn get(&self, local_id: &connection::LocalId) -> Option<Connection> {
        self.mapping.get(local_id).copied()
    }

    pub fn try_insert(
        &mut self,
        local_id: connection::LocalId,
        connection: Connection,
    ) -> Result<(), ()> {
        match self.mapping.entry(local_id) {
            Entry::Occupied(_) => Err(()),
            Entry::Vacant(entry) => {
                entry.insert(connection);
                Ok(())
            }
        }
    }

    pub fn remove(&mut self, local_id: &connection::LocalId) -> Option<Connection> {
        self.mapping.remove(local_id)
    }
}

#[derive(Default, Debug, Resource)]
pub struct InitialIdMap {
    /// Maps from initial id to internal connection ID
    initial_to_internal_id_map: HashMap<connection::InitialId, Connection>,
    /// Maps from internal connection ID to initial ID
    internal_to_initial_id_map: HashMap<Connection, connection::InitialId>,
}

impl InitialIdMap {
    /// Gets the `InternalConnectionId` (if any) associated with the given initial id
    fn get(&self, initial_id: &connection::InitialId) -> Option<Connection> {
        self.initial_to_internal_id_map.get(initial_id).copied()
    }

    /// Inserts the given `InitialId` into the map if it is not already in the map,
    /// otherwise returns an Err
    fn try_insert(
        &mut self,
        initial_id: connection::InitialId,
        internal_id: Connection,
    ) -> Result<(), ()> {
        let initial_to_internal_id_entry = self.initial_to_internal_id_map.entry(initial_id);
        let internal_to_initial_id_entry = self.internal_to_initial_id_map.entry(internal_id);

        match (initial_to_internal_id_entry, internal_to_initial_id_entry) {
            (Entry::Occupied(_), _) | (_, Entry::Occupied(_)) => Err(()),
            (Entry::Vacant(initial_entry), Entry::Vacant(internal_entry)) => {
                initial_entry.insert(internal_id);
                internal_entry.insert(initial_id);
                Ok(())
            }
        }
    }

    /// Removes the `InitialId` associated with the given `InternalConnectionId` from the map
    pub fn remove(&mut self, internal_id: &Connection) -> Option<connection::InitialId> {
        let initial_id = self.internal_to_initial_id_map.remove(internal_id)?;
        self.initial_to_internal_id_map.remove(&initial_id);
        Some(initial_id)
    }
}
