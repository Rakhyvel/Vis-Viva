use apricot::{
    app::App,
    high_precision::WorldPosition,
    ray::Ray,
    rectangle::Rectangle,
    render_core::{LinePathComponent, ModelComponent, RenderContext, TextureId},
    sphere::Sphere,
};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, vec4, Vec2};

use crate::{
    render::{camera::CameraRig, scene::is_occluded},
    sim::{
        bodies::{Body, SurfaceTile, TileMap},
        hierarchy::{Named, Parent},
        propulsion::Craft,
    },
};

/// What's under the mouse, plus the reticles and tile outlines that show it
#[derive(Default)]
pub struct Picker {
    /// Craft, building, or body under the mouse this frame
    hovered: Option<Entity>,
    /// Tile under the mouse on the focused body
    hovered_tile: Option<(Entity, usize, LinePathComponent)>,
    /// Tile under the selected building, or the last bare tile clicked
    selected_tile: Option<(Entity, usize, LinePathComponent)>,
    /// Last bare tile clicked. Stays lit while its body is selected
    clicked_tile_key: Option<(Entity, usize)>,
}

/// A tile on a body, and the outline buffer drawn around it
type TileOutline = (Entity, usize, LinePathComponent);

#[derive(Clone, Copy, PartialEq)]
enum HoverKind {
    Craft,
    Body,
}

impl Picker {
    pub fn update(
        &mut self,
        world: &World,
        rig: &CameraRig,
        selected: Option<Entity>,
        app: &App,
    ) -> Option<Entity> {
        self.hovered = None;
        let craft = self.hover(world, rig, HoverKind::Craft, app);
        let tile = self.hover_tile(world, rig, selected, app);
        let body = self.hover(world, rig, HoverKind::Body, app);
        craft.or(tile).or(body)
    }

    pub fn sync_selected_tile(
        &mut self,
        world: &World,
        selected: Option<Entity>,
        renderer: &RenderContext,
    ) {
        let key = self.selected_tile_key(world, selected);

        if key.is_none() {
            // clear the clicked tile
            self.clicked_tile_key = None;
        }

        let want = key.and_then(|(b, i)| tile_outline_vertices(world, b, i).map(|v| (b, i, v)));
        sync_tile_path(&mut self.selected_tile, want, 1.0, renderer);
    }

    pub fn render(&self, world: &World, rig: &CameraRig, selected: Option<Entity>, app: &App) {
        let reticle = app.renderer.get_texture_id_from_name("reticle").unwrap();

        // A selected craft sits dead center once the camera finishes swooshing to it
        if let Some(entity) = selected {
            if !rig.is_animating(app.seconds as f64) && world.get::<&Craft>(entity).is_ok() {
                let center = vec2(app.window_size.x as f32, app.window_size.y as f32) * 0.5;
                draw_reticle(reticle, center, app);
            }
        }

        // The hovered thin, if it's too small to see and not behind a body
        if let (Some(hovered), Some(selected)) = (self.hovered, selected) {
            if hovered != selected {
                let pos = world.get::<&WorldPosition>(hovered).unwrap().pos;
                let radius = world.get::<&Body>(hovered).map_or(0.0, |b| b.body_radius);
                let relative_pos = pos - rig.world_pos();

                if let Some(screen_pos) = rig.world_to_screen(relative_pos, app) {
                    let too_small = rig.apparent_radius_px(radius, relative_pos.norm(), app) < 2.0;
                    if too_small && !is_occluded(world, rig.world_pos(), hovered, relative_pos) {
                        draw_reticle(reticle, screen_pos, app);
                        let named = world.get::<&Named>(hovered).unwrap();
                        app.renderer
                            .draw_text(screen_pos + vec2(8.0, 8.0), &named.name);
                    }
                }
            }
        }

        self.render_tile_outlines(world, rig, app);
    }

    fn hover(
        &mut self,
        world: &World,
        rig: &CameraRig,
        kind: HoverKind,
        app: &App,
    ) -> Option<Entity> {
        if self.hovered.is_some() {
            return None; // something with higher priority is already hovered
        }

        for (entity, (world_pos, _model)) in world
            .query::<hecs::Without<(&WorldPosition, &ModelComponent), &SurfaceTile>>()
            .iter()
        {
            let body = world.get::<&Body>(entity);
            if body.is_ok() != (kind == HoverKind::Body) {
                continue;
            }
            let relative_pos = world_pos.pos - rig.world_pos();
            let Some(screen_pos) = rig.world_to_screen(relative_pos, app) else {
                continue;
            };

            let radius = body.map_or(0.0, |b| b.body_radius);
            let dist_px = nalgebra_glm::l2_norm(&(screen_pos - app.mouse_pos)) as f64;
            if dist_px
                < rig
                    .apparent_radius_px(radius, relative_pos.norm(), app)
                    .max(16.0)
            {
                self.hovered = Some(entity);
                return take_click(app).then_some(entity);
            }
        }
        None
    }

    fn hover_tile(
        &mut self,
        world: &World,
        rig: &CameraRig,
        selected: Option<Entity>,
        app: &App,
    ) -> Option<Entity> {
        let pick = self.pick_tile(world, rig, selected, app);
        let mut clicked = None;

        if let Some((body, index, _)) = &pick {
            let occupant = world
                .get::<&TileMap>(*body)
                .unwrap()
                .occupant(*index as u32);
            if occupant.is_some() {
                self.hovered = occupant
            }

            // unlike craft and bodies, a click on a bare tile is remembered but not consumed
            if app.mouse_left_clicked && !app.is_click_consumed() {
                self.clicked_tile_key = Some((*body, *index));
                if occupant.is_some() {
                    clicked = occupant;
                    app.consume_click();
                }
            }
        }

        sync_tile_path(&mut self.hovered_tile, pick, 0.45, &app.renderer);
        clicked
    }

    fn pick_tile(
        &self,
        world: &World,
        rig: &CameraRig,
        selected: Option<Entity>,
        app: &App,
    ) -> Option<(Entity, usize, Vec<f32>)> {
        if self.hovered.is_some() {
            return None; // something with higher priority is already hovered
        }

        let selected = selected?;
        let body_entity = if world.get::<&Body>(selected).is_ok() {
            selected
        } else {
            world.get::<&Parent>(selected).ok()?.id
        };

        let mut q = world
            .query_one::<(&Body, &WorldPosition, &TileMap)>(body_entity)
            .ok()?;
        let (body, pos, tile_map) = q.get()?;
        if body.gaseous() {
            return None; // gas giants don't have tiles
        }

        // Is the mouse over the body at all?
        let center = nalgebra_glm::convert(pos.pos - rig.world_pos());
        let sphere = Sphere {
            center,
            radius: body.body_radius as f32,
        };
        let mouse_ray = rig.mouse_ray(app);
        sphere.raycast(&mouse_ray)?;

        // Nearest tile the ray hits, in the body's unit-sphere space
        let r = body.body_radius as f32 * 1.002;
        let local_ray = Ray::new((mouse_ray.origin() - center) / r, mouse_ray.dir());
        let (index, _) = tile_map
            .tris
            .iter()
            .enumerate()
            .filter_map(|(i, tri)| tri.raycast(&local_ray).map(|t| (i, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;

        Some((
            body_entity,
            index,
            tile_outline_vertices(world, body_entity, index)?,
        ))
    }

    fn selected_tile_key(
        &self,
        world: &World,
        selected: Option<Entity>,
    ) -> Option<(Entity, usize)> {
        let sel = selected?;

        if let (Ok(tile), Ok(parent)) = (world.get::<&SurfaceTile>(sel), world.get::<&Parent>(sel))
        {
            return Some((parent.id, tile.index as usize));
        }

        self.clicked_tile_key.filter(|(body, _)| *body == sel)
    }

    fn render_tile_outlines(&self, world: &World, rig: &CameraRig, app: &App) {
        let (view, proj) = rig.camera().inner.view_proj_matrices();

        for (body, index, outline) in [&self.selected_tile, &self.hovered_tile]
            .into_iter()
            .flatten()
        {
            let relative_pos = world.get::<&WorldPosition>(*body).unwrap().pos - rig.world_pos();
            let d = relative_pos.norm();
            let r = world.get::<&Body>(*body).unwrap().body_radius;
            let tile_dir = world
                .get::<&TileMap>(*body)
                .unwrap()
                .tile_offset(*index as u32, 1.0);

            if tile_dir.dot(&(-relative_pos / d)) <= r / d {
                continue; // tile faces away from the camera
            }

            app.renderer.draw_line_path_at(
                outline,
                nalgebra_glm::convert(relative_pos),
                view,
                proj,
            );
        }
    }
}

/// Claim this frame's left click if nobody else has
fn take_click(app: &App) -> bool {
    let clicked = app.mouse_left_clicked && !app.is_click_consumed();
    if clicked {
        app.consume_click();
    }
    clicked
}

/// The 16 px reticle, centred on `center`
fn draw_reticle(texture: TextureId, center: Vec2, app: &App) {
    const SIZE: f32 = 16.0;
    app.renderer.copy_texture(
        Rectangle::new(center.x - SIZE * 0.5, center.y - SIZE * 0.5, SIZE, SIZE),
        texture,
        Rectangle::new(0.0, 0.0, SIZE, SIZE),
        &vec4(1.0, 1.0, 1.0, 1.0),
    );
}

/// Keep `slot`'s outline buffer matching `want`, only rebuilding it when the tile changes
fn sync_tile_path(
    slot: &mut Option<TileOutline>,
    want: Option<(Entity, usize, Vec<f32>)>,
    alpha: f32,
    renderer: &RenderContext,
) {
    let same = match (&*slot, &want) {
        (Some((e, i, _)), Some((ne, ni, _))) => e == ne && i == ni,
        (None, None) => true,
        _ => false,
    };
    if same {
        return; // same tile as last frame, keep the buffer we already have
    }

    if let Some((_, _, mut lp)) = slot.take() {
        lp.queue_deletion(renderer);
    }

    *slot = want.map(|(e, i, v)| {
        let mut line_path = LinePathComponent::new(v);
        line_path.color = vec4(1.0, 1.0, 1.0, alpha);
        line_path.width = 5.0;
        line_path.fade = false;
        line_path.depth_test = false;
        (e, i, line_path)
    });
}

/// A closed outline around a tile, just above the body's surface
fn tile_outline_vertices(world: &World, body: Entity, index: usize) -> Option<Vec<f32>> {
    let mut q = world.query_one::<(&Body, &TileMap)>(body).ok()?;
    let (b, tile_map) = q.get()?;

    let r = b.body_radius as f32 * 1.002;
    let corners: [f32; 9] = (*tile_map.tris.get(index)? * r).into();

    let mut v = Vec::with_capacity(12);
    v.extend_from_slice(&corners);
    v.extend_from_slice(&corners[0..3]);
    Some(v)
}
