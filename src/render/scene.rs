use apricot::{
    app::App,
    bvh::{BVHNodeId, BVH},
    camera::{Camera, ProjectionKind},
    high_precision::WorldPosition,
    rectangle::Rectangle,
    render_core::{MeshId, ModelComponent, RenderContext, TextureId},
    shadow_map::DirectionalLightSource,
};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, vec3, vec4, DVec3, Vec3};

use crate::{
    render::{camera::CameraRig, starbox::Starbox},
    sim::bodies::{Body, Category, SurfaceTile, TileClass},
};

pub struct SceneRenderer {
    /// The sun's light source
    light: DirectionalLightSource,
    bvh: BVH<Entity>,
    starbox: Starbox,
}

pub struct SceneObject {
    pub bvh_node_id: Option<BVHNodeId>,
}

impl SceneRenderer {
    pub fn new(bvh: BVH<Entity>) -> Self {
        Self {
            light: DirectionalLightSource::new(
                Camera::new(
                    vec3(0.0, 0.0, 0.0),
                    vec3(0.0, 10.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    ProjectionKind::Orthographic {
                        // These do not matter for now, they're reset later
                        left: 0.0,
                        right: 0.0,
                        bottom: 0.0,
                        top: 0.0,
                        near: 0.0,
                        far: 0.0,
                    },
                    4.0 / 3.0,
                ),
                vec3(-1.0, 0.0, 0.0),
                1024,
            ),
            bvh,
            starbox: Starbox::new(9000, vec3(1.0, 2.0, 4.0), 0.4),
        }
    }

    pub fn bvh_mut(&mut self) -> &mut BVH<Entity> {
        &mut self.bvh
    }

    /// Draw that splish yo
    pub fn render(&mut self, world: &mut World, rig: &CameraRig, app: &App) {
        // Clear
        self.light.light_dir = -nalgebra_glm::convert::<DVec3, Vec3>(rig.world_pos());
        app.renderer.set_camera(rig.camera().inner);

        app.renderer.set_color(vec4(0.01, 0.01, 0.01, 1.0));
        app.renderer.clear();
        self.starbox.draw(app);
        draw_dots(world, rig, app);
        app.renderer
            .directional_light_system(&mut self.light, world, &self.bvh);
        app.renderer.render_3d_models_system(
            world,
            &self.light,
            &self.bvh,
            Some(rig.camera()),
            false,
        );
        app.renderer.render_3d_line_paths(world, Some(rig.camera()));
    }

    pub fn sync_models(&mut self, world: &mut World, camera_pos: DVec3, app: &App) {
        for (_entity, (world_pos, model, scene_obj)) in
            world.query_mut::<(&WorldPosition, &mut ModelComponent, &SceneObject)>()
        {
            let new_pos: Vec3 = nalgebra_glm::convert(world_pos.pos - camera_pos);
            model.set_position(new_pos);
            self.bvh.move_obj(
                scene_obj.bvh_node_id.unwrap(),
                &app.renderer.get_model_aabb(model),
                &vec3(0.0f32, 0.0, 0.0),
            );
        }
    }
}

fn draw_dots(world: &World, rig: &CameraRig, app: &App) {
    app.renderer.set_color(vec4(1.0, 1.0, 1.0, 1.0));

    for (entity, (world_pos, _model)) in world
        .query::<hecs::Without<(&WorldPosition, &ModelComponent), &SurfaceTile>>()
        .iter()
    {
        let relative_pos = world_pos.pos - rig.world_pos();
        if let Some(screen) = rig.world_to_screen(relative_pos, app) {
            let rect = Rectangle {
                pos: screen,
                size: vec2(2.0, 2.0),
            };
            let radius = world
                .get::<&Body>(entity)
                .map(|b| b.body_radius)
                .unwrap_or(0.0);

            if rig.apparent_radius_px(radius, relative_pos.norm(), app) < 2.0
                && !is_occluded(world, rig.world_pos(), entity, relative_pos)
            {
                app.renderer.fill_rect(rect);
            }
        }
    }
}

pub fn is_occluded(world: &World, camera_pos: DVec3, entity: Entity, relative_pos: DVec3) -> bool {
    let dist = relative_pos.norm();
    let dir = relative_pos / dist;

    for (other, (opos, obody)) in world.query::<(&WorldPosition, &Body)>().iter() {
        if other == entity {
            continue; // body never occludes itself
        }

        let c = opos.pos - camera_pos;
        let along = c.dot(&dir);
        if along <= 0.0 || along >= dist {
            continue; // behind camera, or further away than the target
        }

        let perp_sq = c.norm_squared() - along * along;
        if perp_sq < obody.body_radius * obody.body_radius {
            return true;
        }
    }
    false
}

pub fn attach_body_model(
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
    entity: Entity,
) {
    let body = *world.get::<&Body>(entity).unwrap();
    let mesh_name = match body.tile_class() {
        None => "uv",
        Some(TileClass::Dwarf) => "ico-20",
        Some(TileClass::Sub) => "ico-80",
        Some(TileClass::Large) => "ico-320",
    };
    let mesh_id = renderer.get_mesh_id_from_name(mesh_name).unwrap();
    let texture_id = body_texture(&body, renderer);
    let r = body.body_radius;
    attach_model(
        world,
        renderer,
        bvh,
        entity,
        mesh_id,
        texture_id,
        vec3(0.01, 0.01, 0.01),
    );
}

pub fn attach_craft_model(
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
    craft: Entity,
) {
    let mesh_id = renderer.get_mesh_id_from_name("cone").unwrap();
    let texture_id = renderer.get_texture_id_from_name("europa").unwrap();
    attach_model(
        world,
        renderer,
        bvh,
        craft,
        mesh_id,
        texture_id,
        vec3(0.01, 0.01, 0.01),
    );
}

/// Insert a ModelComponent and a BVH node for `entity`.
fn attach_model(
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
    entity: Entity,
    mesh_id: MeshId,
    texture_id: TextureId,
    scale_vec: DVec3,
) {
    let position: DVec3 = vec3(0., 0., 0.);

    let bvh_node_id = bvh.insert(
        entity,
        renderer
            .get_mesh_aabb(mesh_id)
            .scale(nalgebra_glm::convert(scale_vec))
            .translate(nalgebra_glm::convert(position)),
    );

    world
        .insert(
            entity,
            (
                ModelComponent::new(
                    mesh_id,
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

fn body_texture(body: &Body, renderer: &RenderContext) -> TextureId {
    if body.category == Category::Star {
        return renderer.get_texture_id_from_name("sun").unwrap();
    }

    if body.gaseous() {
        if !body.is_giant() {
            renderer.get_texture_id_from_name("venus").unwrap()
        } else if body.temperature > 120.0 {
            renderer.get_texture_id_from_name("jupiter").unwrap()
        } else {
            renderer.get_texture_id_from_name("uranus").unwrap()
        }
    } else {
        if body.habitable() {
            renderer.get_texture_id_from_name("earth").unwrap()
        } else if body.temperature < 200.0 {
            renderer.get_texture_id_from_name("europa").unwrap()
        } else {
            renderer.get_texture_id_from_name("moon").unwrap()
        }
    }
}
