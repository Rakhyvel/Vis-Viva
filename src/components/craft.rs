use apricot::{
    bvh::BVH,
    high_precision::WorldPosition,
    render_core::{LinePathComponent, ModelComponent, RenderContext},
};
use hecs::{Entity, World};
use nalgebra_glm::{vec3, DVec3};

use crate::{
    astro::state::State,
    components::body::SceneObject,
    sim::{bodies::Body, hierarchy::ParentBody, propulsion::Craft},
};

pub struct AssociatedEntity {
    pub associate: Entity,
}

pub fn replace_line_path(
    world: &mut World,
    renderer: &RenderContext,
    craft_entity: Entity,
    new_line_path: Option<(
        WorldPosition,
        ParentBody,
        LinePathComponent,
        AssociatedEntity,
    )>,
) {
    let old_line_path = world.get::<&Craft>(craft_entity).unwrap().line_path_entity;

    if let Some(old) = old_line_path {
        {
            let mut line_path = world.get::<&mut LinePathComponent>(old).unwrap();
            line_path.queue_deletion(renderer);
        }
        world.despawn(old).ok();
    }

    let new_entity = new_line_path.map(|components| world.spawn(components));
    world
        .get::<&mut Craft>(craft_entity)
        .unwrap()
        .line_path_entity = new_entity;
}

pub fn redraw_orbit(
    world: &mut World,
    renderer: &RenderContext,
    craft: Entity,
    soi_radius: Option<f64>,
) {
    let state = *world.get::<&State>(craft).unwrap();
    let parent = world.get::<&ParentBody>(craft).unwrap().id;
    let parent_pos = world.get::<&WorldPosition>(parent).unwrap().pos;
    let parent_mu = world.get::<&Body>(parent).unwrap().mu;

    let vertices: Vec<f32> = state
        .generate_orbit_vertices(8192, parent_mu, soi_radius)
        .unwrap()
        .iter()
        .flat_map(|v| v.iter().map(|x| *x as f32))
        .collect();

    replace_line_path(
        world,
        renderer,
        craft,
        Some((
            WorldPosition { pos: parent_pos },
            ParentBody { id: parent },
            LinePathComponent::new(vertices),
            AssociatedEntity { associate: craft },
        )),
    );
}
