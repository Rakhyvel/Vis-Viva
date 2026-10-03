use std::f64::consts::PI;

use apricot::{
    app::App,
    camera::{Camera, ProjectionKind},
    high_precision::{self, WorldPosition},
    ray::Ray,
};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, vec3, vec4, DVec3, Vec2};

use crate::sim::{bodies::SurfaceTile, hierarchy::Parent};

pub struct CameraRig {
    /// The camera used for rendering 3d models
    camera: high_precision::Camera,
    /// Side-side view angle
    theta: f64,
    /// Up-down view angle
    phi: f64,
    /// How far the camera sits from the focus point
    distance: f64,

    /// Where the camera is looking
    focus: DVec3,
    /// Where the focus was when the selection last changed.
    prev_focus: DVec3,
    /// App time the selection last changed
    transition_start: f64,
}

impl CameraRig {
    pub fn new() -> Self {
        Self {
            camera: high_precision::Camera {
                world_pos: vec3(1.0, 1.0, 1.0),
                inner: Camera::new(
                    vec3(1.0, 0.0, 1.0),
                    vec3(0., 0.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    apricot::camera::ProjectionKind::Perspective {
                        fov_rad: 67.0f32.to_radians(),
                        far: 10000000.0,
                    },
                    4.0 / 3.0,
                ),
            },
            phi: 2.5,
            theta: -PI / 4.0,
            distance: 64.0,
            focus: vec3(0.0, 0.0, 0.0),
            prev_focus: vec3(0.0, 0.0, 0.0),
            transition_start: f64::NEG_INFINITY,
        }
    }

    /// Follow `target`, swooshing over 1s whenever the selection changes
    pub fn update(&mut self, target: Option<DVec3>, selection_changed_at: f64, app: &App) {
        if selection_changed_at != self.transition_start {
            self.prev_focus = self.focus;
            self.transition_start = selection_changed_at
        }
        if let Some(target) = target {
            self.focus = target
        }

        let rot = nalgebra_glm::rotate_y(
            &nalgebra_glm::rotate_z(&nalgebra_glm::one(), self.phi),
            self.theta,
        );
        let t = cubic_ease_in_out((app.seconds as f64 - self.transition_start).min(1.0));
        let offset = (1.0 - t) * self.prev_focus + t * self.focus;
        self.camera.world_pos = (rot * vec4(self.distance, 0., 0., 0.)).xyz() + offset;
        self.camera.sync(offset);
    }

    /// Drag to swivel, scroll to zoom
    pub fn orbit_controls(&mut self, app: &App, body_radius: f64) {
        let altitude = self.distance - body_radius;

        let min_distance: f64 = 0.12 + body_radius;
        let max_distance: f64 = 1e6 + body_radius;

        let control_speed = 0.0005 * (altitude - min_distance).clamp(4.0, 10.0);
        if app.mouse_left_dragging {
            self.phi -= control_speed * (app.mouse_vel.x as f64);
            self.theta = (self.theta - control_speed * (app.mouse_vel.y as f64))
                .max(control_speed - PI / 2.0)
                .min(PI / 2.0 - control_speed);
        }

        if !app.is_wheel_consumed() {
            app.consume_wheel();
            let zoom_factor = 0.95f64.powf(app.mouse_wheel as f64);
            self.distance = (self.distance * zoom_factor).clamp(min_distance, max_distance);
        }
    }

    pub fn is_animating(&self, app_seconds: f64) -> bool {
        app_seconds - self.transition_start < 1.0
    }

    pub fn sync_aspect(&mut self, app: &App) {
        let aspect = app.window_size.x as f32 / app.window_size.y as f32;
        if (self.camera.inner.aspect_ratio() - aspect).abs() > 1e-6 {
            self.camera.inner.set_aspect_ratio(aspect);
        }
    }

    pub fn camera(&self) -> &high_precision::Camera {
        &self.camera
    }

    pub fn world_pos(&self) -> DVec3 {
        self.camera.world_pos
    }

    pub fn mouse_ray(&self, app: &App) -> Ray {
        self.camera.inner.get_ray(
            app.mouse_pos.x,
            app.mouse_pos.y,
            app.window_size.x as f32,
            app.window_size.y as f32,
        )
    }

    pub fn world_to_screen(&self, relative_pos: DVec3, app: &App) -> Option<Vec2> {
        let window_size = app.window_size;
        let (view, proj) = self.camera.inner.view_proj_matrices();
        let clip = proj
            * view
            * vec4(
                relative_pos.x as f32,
                relative_pos.y as f32,
                relative_pos.z as f32,
                1.0,
            );
        if clip.w <= 0.0 {
            return None; // behind camera
        }
        let ndc = clip.xyz() / clip.w;
        Some(vec2(
            ((ndc.x + 1.0) / 2.0) as f32 * window_size.x as f32,
            ((1.0 - ndc.y) / 2.0) as f32 * window_size.y as f32,
        ))
    }

    /// Radius of a body on screen in px
    pub fn apparent_radius_px(&self, radius: f64, dist: f64, app: &App) -> f64 {
        let ProjectionKind::Perspective { fov_rad, .. } = self.camera.inner.projection_kind else {
            return 0.0;
        };
        (radius / dist) / (fov_rad as f64 / 2.0).tan() * (app.window_size.y as f64 / 2.0)
    }
}

pub fn focus_point(world: &World, selected: Option<Entity>) -> Option<DVec3> {
    let Some(selected_entity) = selected else {
        return None;
    };

    // Buildings keep the camera centered on their planet, not on themselves
    let focus = if world.get::<&SurfaceTile>(selected_entity).is_ok() {
        world
            .get::<&Parent>(selected_entity)
            .map(|p| p.id)
            .unwrap_or(selected_entity)
    } else {
        selected_entity
    };

    if let Ok(world_pos) = world.get::<&WorldPosition>(focus) {
        return Some(world_pos.pos);
    }

    None
}

/// Cubic easing out function for animations
fn cubic_ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powf(3.0) / 2.0
    }
}
