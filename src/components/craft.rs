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
    sim::{bodies::Body, hierarchy::Parent, propulsion::Craft},
};

pub struct AssociatedEntity {
    pub associate: Entity,
}

/// Give a sim-spawned craft its model and BVH node
pub fn attach_craft_model(
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
    craft: Entity,
) {
    let craft_mesh = renderer.get_mesh_id_from_name("cone").unwrap();
    let texture_id = renderer.get_texture_id_from_name("europa").unwrap();
    let scale_vec: DVec3 = vec3(0.01, 0.01, 0.01);
    let position: DVec3 = vec3(0., 0., 0.);

    let bvh_node_id = bvh.insert(
        craft,
        renderer
            .get_mesh_aabb(craft_mesh)
            .scale(nalgebra_glm::convert(scale_vec))
            .translate(nalgebra_glm::convert(position)),
    );

    world
        .insert(
            craft,
            (
                ModelComponent::new(
                    craft_mesh,
                    texture_id,
                    nalgebra_glm::convert(position),
                    nalgebra_glm::convert(scale_vec),
                ),
                SceneObject {
                    bvh_node_id: Some(bvh_node_id),
                },
            ),
        )
        .unwrap();
}

pub fn replace_line_path(
    world: &mut World,
    renderer: &RenderContext,
    craft_entity: Entity,
    new_line_path: Option<(WorldPosition, Parent, LinePathComponent, AssociatedEntity)>,
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
    let parent = world.get::<&Parent>(craft).unwrap().id;
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
            Parent { id: parent },
            LinePathComponent::new(vertices),
            AssociatedEntity { associate: craft },
        )),
    );
}
