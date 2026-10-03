//! This module is responsible for defining the body component

use std::collections::HashMap;

use apricot::{
    bvh::{BVHNodeId, BVH},
    high_precision::WorldPosition,
    render_core::{LinePathComponent, ModelComponent, RenderContext, TextureId},
};
use hecs::{Entity, World};
use nalgebra_glm::{vec3, DVec3};

use crate::{
    astro::state::State,
    render::orbit_lines::AssociatedEntity,
    sim::{
        bodies::{Body, Category, TileMap, TileSets},
        hierarchy::{Named, ParentBody},
        parts::PartInventory,
    },
};

pub struct SceneObject {
    pub bvh_node_id: Option<BVHNodeId>,
}

pub fn spawn_body(
    body: Body,
    init_state: State,
    mut scene_obj: SceneObject,
    named: Named,
    parent: Option<ParentBody>,
    tile_sets: &TileSets,
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
) -> Entity {
    // choose the tile set, roughly scaling tile area linearly
    const MARS_RADIUS: f64 = 0.532;
    let (body_mesh, tiles) = if body.gaseous() {
        (renderer.get_mesh_id_from_name("uv").unwrap(), None)
    } else if body.body_radius > MARS_RADIUS {
        (
            renderer.get_mesh_id_from_name("ico-320").unwrap(),
            Some(&tile_sets.large),
        )
    } else if body.body_radius > MARS_RADIUS * 0.5 {
        (
            renderer.get_mesh_id_from_name("ico-80").unwrap(),
            Some(&tile_sets.sub),
        )
    } else {
        (
            renderer.get_mesh_id_from_name("ico-20").unwrap(),
            Some(&tile_sets.dwarf),
        )
    };

    let position: DVec3 = vec3(0., 0., 0.);
    let scale_vec: DVec3 = vec3(body.body_radius, body.body_radius, body.body_radius);

    let texture_id = body.get_texture_id(renderer);

    let body_entity = world.spawn((
        WorldPosition { pos: position },
        ModelComponent::new(
            body_mesh,
            texture_id,
            nalgebra_glm::convert(position),
            nalgebra_glm::convert(scale_vec),
        ),
    ));

    if let Some(parent) = parent {
        let parent_world_pos = world.get::<&WorldPosition>(parent.id).unwrap().pos;
        let parent_mu = { world.get::<&Body>(parent.id).unwrap().mu };
        let vertices: Vec<f32> = init_state
            .generate_orbit_vertices(8192, parent_mu, None)
            .unwrap()
            .iter()
            .flat_map(|v| v.iter().map(|x| *x as f32))
            .collect();

        let _line_path_entity = world.spawn((
            WorldPosition {
                pos: parent_world_pos,
            },
            parent,
            LinePathComponent::new(vertices),
            AssociatedEntity {
                associate: body_entity,
            },
        ));
        world.insert(body_entity, (parent,)).unwrap();
    }

    let bvh_node_id = bvh.insert(
        body_entity,
        renderer
            .get_mesh_aabb(body_mesh)
            .scale(nalgebra_glm::convert(scale_vec))
            .translate(nalgebra_glm::convert(position)),
    );

    scene_obj.bvh_node_id = Some(bvh_node_id);

    world
        .insert(
            body_entity,
            (
                scene_obj,
                named,
                init_state,
                body,
                PartInventory {
                    parts: HashMap::new(),
                },
                TileMap::new(tiles.cloned().unwrap_or_default()),
            ),
        )
        .unwrap();

    body_entity
}

impl Body {
    fn get_texture_id(&self, renderer: &RenderContext) -> TextureId {
        if self.category == Category::Star {
            return renderer.get_texture_id_from_name("sun").unwrap();
        }

        if self.gaseous() {
            if !self.is_giant() {
                renderer.get_texture_id_from_name("venus").unwrap()
            } else if self.temperature > 120.0 {
                renderer.get_texture_id_from_name("jupiter").unwrap()
            } else {
                renderer.get_texture_id_from_name("uranus").unwrap()
            }
        } else {
            if self.habitable() {
                renderer.get_texture_id_from_name("earth").unwrap()
            } else if self.temperature < 200.0 {
                renderer.get_texture_id_from_name("europa").unwrap()
            } else {
                renderer.get_texture_id_from_name("moon").unwrap()
            }
        }
    }
}
