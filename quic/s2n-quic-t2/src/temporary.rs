use crate::prelude::*;

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct Marker;

#[derive(Clone, Copy, Debug, StageLabel)]
pub struct Label;

pub fn schedule(s: &mut Schedule) {
    s.add_stage(Label, SystemStage::single(cleanup));
}

pub fn cleanup(mut query: Query<(Entity, &Marker)>, mut commands: Commands) {
    for (entity, _) in &mut query {
        commands.entity(entity).despawn();
    }
}
