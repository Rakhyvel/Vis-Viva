//! This module is responsible for defining the gameplay scene.

use std::{cell::Cell, collections::HashMap, f64::consts::PI, rc::Rc};

use apricot::{
    app::{App, Scene},
    bvh::BVH,
    camera::{Camera, ProjectionKind},
    high_precision::{self, WorldPosition},
    opengl::create_program,
    ray::Ray,
    rectangle::Rectangle,
    render_core::{LinePathComponent, ModelComponent, RenderContext},
    shadow_map::DirectionalLightSource,
    sphere::Sphere,
};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, vec3, vec4, DVec3, Vec2, Vec3};
use sdl2::keyboard::Scancode;

use crate::{
    astro::{
        epoch::EphemerisTime,
        maneuver::sphere_of_influence,
        state::State,
        units::{METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR, SUN_MU},
    },
    components::craft::{replace_line_path, spawn_craft, AssociatedEntity},
    container,
    generation::{lexicon::Lexicon, polygon},
    hud::{
        fabricator::{FabricatorAction, FabricatorUi},
        footer::{Footer, FooterView, TurnMessages},
        game_over::GameOverUi,
        maneuver::ManeuverModal,
        panel::{self, panel_structure_bits, CommandMessages, PanelCtx},
        sim_speed::SimSpeed,
        timeline::{MarkKind, TimelineMark},
        transfer::{TransferResult, TransferUi},
        Binding,
    },
    scenes::starbox::Starbox,
    sim::{
        bodies::{Body, Category, SurfaceTile, TileMap, TileSets},
        docking::{allocate_ports, next_free_port, Docking, PortHost},
        events::{Event, EventQueue},
        hierarchy::{
            docked_position_system, get_ancestor, landed_system, orbit_system, Landed, Named,
            Parent,
        },
        industry::{commit_pending_builds, projected_completion, Factory},
        life_support::{crew_death, Station},
        mission::Command,
        parts::{id_hash, ModuleSpec, PartDef, PartInventory, PartRegistry},
        propulsion::{apply_burn, Craft},
        resources::{
            add_resource, commit_station, next_reservoir_limits, transfer_resource, Electrolyzer,
            Miner, Resource, ResourceStore, SolarPanel,
        },
    },
    ui::{
        anchor::{Anchor, AnchorPoint},
        style::STYLE,
    },
};

use crate::{
    components::{
        body::{spawn_body, SceneObject},
        icosphere,
    },
    generation::solar_system_gen::{self},
    ui::{
        container::Container,
        widget::{recv_msgs, Widget},
    },
};

/// Object file data, used for meshes
pub const QUAD_XY_DATA: &[u8] = include_bytes!("../../res/quad-xy.obj");
pub const UV_DATA: &[u8] = include_bytes!("../../res/uv-sphere.obj");
pub const CONE_DATA: &[u8] = include_bytes!("../../res/cone.obj");
pub const CUBE_DATA: &[u8] = include_bytes!("../../res/cube.obj");

/// Struct that contains info about the game state
pub struct Gameplay {
    /// The world where all the entities live
    world: World,
    /// The camera used for rendering 3d models
    camera_3d: high_precision::Camera,
    /// The sun's light source
    directional_light: DirectionalLightSource,
    /// A bounding-volume hierarchy, a container that stores models and allows for efficient lookup for fast rendering
    bvh: BVH<Entity>,

    selection: SelectionState,
    hovered: Option<Entity>,
    selected_tile: Option<(Entity, usize, LinePathComponent)>,
    clicked_tile_key: Option<(Entity, usize)>,
    hovered_tile: Option<(Entity, usize, LinePathComponent)>,

    /// All the parts, loaded from the toml
    parts: PartRegistry,

    /// Up-down view angle
    phi: f64,
    /// Side-side view angle
    theta: f64,
    /// How far the camera swivels around the currently selected body
    distance: f64,

    /// Used for tab key latch
    prev_tab_state: bool,

    footer: Footer,
    gui: Anchor<CommandMessages>,
    gui_built_for: Option<(Entity, u32, u64, u64)>,
    gui_bindings: Vec<Binding>,
    fabricator_ui: FabricatorUi,
    maneuver_ui: ManeuverModal,
    transfer_ui: TransferUi,
    game_over_ui: GameOverUi,

    /// Buttons in the side panel are only clickable while paused
    controls_enabled: Rc<Cell<bool>>,

    // Events and timeline
    event_queue: EventQueue,
    current_et: Rc<Cell<EphemerisTime>>,
    paused: bool,
    sim_speed: SimSpeed,
    /// Either the next event, or None
    run_until: Option<EphemerisTime>,

    // Vec of unit vectors
    starbox: Starbox,
}

#[derive(Debug)]
enum SelectionKind {
    Craft,
    Body,
    Building,
}

struct SelectionState {
    pub crafts: Vec<Entity>,
    pub bodies: Vec<Entity>,
    pub buildings: Vec<Entity>,

    pub selected: Option<usize>,
    pub kind: SelectionKind,

    // For swoosh animation
    pub selected_pos: DVec3,
    pub prev_selected_pos: DVec3,
    pub transition: f64,
}

impl SelectionState {
    pub fn new(crafts: Vec<Entity>, bodies: Vec<Entity>, buildings: Vec<Entity>) -> Self {
        Self {
            crafts,
            bodies,
            buildings,
            selected: None,
            kind: SelectionKind::Body,
            selected_pos: vec3(0.0, 0.0, 0.0),
            prev_selected_pos: vec3(0.0, 0.0, 0.0),
            transition: 0.0,
        }
    }

    pub fn selected_entity(&self) -> Option<Entity> {
        self.selected.map(|s| self.curr_sel_track()[s])
    }

    pub fn set_selected(&mut self, entity: Entity, app_seconds: f64) {
        if let Some(selected) = self.selected_entity() {
            if selected == entity {
                return;
            }
        }

        let found = self
            .crafts
            .iter()
            .position(|e| *e == entity)
            .map(|x| (x, SelectionKind::Craft))
            .or(self
                .bodies
                .iter()
                .position(|e| *e == entity)
                .map(|x| (x, SelectionKind::Body)))
            .or(self
                .buildings
                .iter()
                .position(|e| *e == entity)
                .map(|x| (x, SelectionKind::Building)));

        if let Some((idx, kind)) = found {
            self.selected = Some(idx);
            self.kind = kind;

            self.prev_selected_pos = self.selected_pos;
            self.transition = app_seconds;
        }
    }

    pub fn prev(&mut self, app_seconds: f64) {
        if let Some(selected) = self.selected {
            let mut new_selection = selected;
            if selected == 0 {
                new_selection = self.curr_sel_track().len() - 1;
            } else {
                new_selection -= 1;
            }
            self.selected = Some(new_selection);
        } else {
            self.selected = Some(0);
        }

        self.prev_selected_pos = self.selected_pos;
        self.transition = app_seconds;
    }

    pub fn next(&mut self, app_seconds: f64) {
        if let Some(selected) = self.selected {
            let mut new_selection = selected + 1;
            if new_selection >= self.curr_sel_track().len() {
                new_selection = 0;
            }
            self.selected = Some(new_selection);
        } else {
            self.selected = Some(0);
        }

        self.prev_selected_pos = self.selected_pos;
        self.transition = app_seconds;
    }

    pub fn is_animating(&self, app_seconds: f64) -> bool {
        app_seconds - self.transition < 1.0
    }

    fn curr_sel_track(&self) -> &Vec<Entity> {
        match self.kind {
            SelectionKind::Body => &self.bodies,
            SelectionKind::Craft => &self.crafts,
            SelectionKind::Building => &self.buildings,
        }
    }
}

impl Scene for Gameplay {
    /// Update the scene every tick
    fn update(&mut self, app: &App) {
        let modal_open = self.fabricator_ui.is_shown()
            || self.maneuver_ui.is_shown()
            || self.transfer_ui.is_shown()
            || self.game_over_ui.is_shown();

        if self.game_over_ui.update(app) {
            app.running.set(false);
        }

        if let Some(FabricatorAction {
            fabricator,
            part_id,
        }) = self.fabricator_ui.update(app)
        {
            let host = self.world.get::<&Docking>(fabricator).unwrap().host;
            let ports = self.parts.get(part_id).map_or(0, |d| d.cost.ports_required);

            let reserved_port = if ports > 0 {
                next_free_port(&self.world, host)
            } else {
                None
            };

            let mut factory = self.world.get::<&mut Factory>(fabricator).unwrap();
            factory.pending_job = Some(part_id);
            factory.reserved_port = reserved_port;
        }

        let now = self.current_et.get();

        if let Some(command) = self.maneuver_ui.update(now, &self.world, app) {
            if let Some(selected) = self.selection.selected_entity() {
                self.world.get::<&mut Craft>(selected).unwrap().command = Some(command);
            }
        }

        if let Some(TransferResult { from, to }) = self.transfer_ui.update(now, &self.world, app) {
            self.commit_station();
            transfer_resource(
                &self.world,
                from.host,
                to.host,
                from.resource,
                f32::MAX,
                now,
            );
            self.transfer_ui.rebuild(now, &self.world, app);
        }

        if !modal_open {
            // Handle all the messages from UI
            for msg in recv_msgs(app, &mut self.gui) {
                match msg {
                    CommandMessages::OpenFabricator { fabricator_entity } => {
                        self.fabricator_ui.show(
                            &self.world,
                            fabricator_entity,
                            &self.parts,
                            self.current_et.get(),
                            app,
                        );
                    }
                    CommandMessages::CancelQueuedFabricator { fabricator_entity } => {
                        let mut factory =
                            self.world.get::<&mut Factory>(fabricator_entity).unwrap();
                        factory.pending_job = None;
                        factory.reserved_port = None;
                    }
                    CommandMessages::CancelActiveFabricator { fabricator_entity } => {
                        let mut factory =
                            self.world.get::<&mut Factory>(fabricator_entity).unwrap();
                        factory.current_job = None;
                        factory.reserved_port = None;
                    }
                    CommandMessages::ToggleFabricator { fabricator_entity } => {
                        self.commit_station();
                        let now = self.current_et.get();
                        let mut factory =
                            self.world.get::<&mut Factory>(fabricator_entity).unwrap();
                        let enabled = factory.enabled;
                        let power = factory.power_watts;
                        if let Some(job) = &mut factory.current_job {
                            if enabled {
                                // bank the energy done before turning off the thing
                                let dt = (now - job.energy_et).as_secs() as f32;
                                job.energy_done =
                                    (job.energy_done + power * dt).min(job.energy_total);
                            }
                            job.energy_et = now;
                        }
                        factory.enabled = !factory.enabled;
                    }
                    CommandMessages::CancelCommand { craft } => {
                        let mut craft = self.world.get::<&mut Craft>(craft).unwrap();
                        craft.command = None;
                    }
                    CommandMessages::ToggleElectrolyzer {
                        electrolyzer_entity,
                    } => {
                        self.commit_station();
                        let mut electrolyzer = self
                            .world
                            .get::<&mut Electrolyzer>(electrolyzer_entity)
                            .unwrap();
                        electrolyzer.enabled = !electrolyzer.enabled;
                    }
                    CommandMessages::ToggleMiner { miner_entity } => {
                        self.commit_station();
                        let mut miner = self.world.get::<&mut Miner>(miner_entity).unwrap();
                        miner.enabled = !miner.enabled;
                    }
                    CommandMessages::Undock { entity } => {
                        self.undock(entity, app);
                    }
                    CommandMessages::SelectEntity { entity } => {
                        self.selection.set_selected(entity, app.seconds as f64);
                    }
                    CommandMessages::OpenManeuver => {
                        if let Some(selected) = self.selection.selected_entity() {
                            self.maneuver_ui.show(
                                selected,
                                self.current_et.get(),
                                &self.world,
                                app,
                            );
                        }
                    }
                    CommandMessages::OpenTransfer => {
                        if let Some(selected) = self.selection.selected_entity() {
                            self.transfer_ui.show(
                                selected,
                                self.current_et.get(),
                                &self.world,
                                app,
                            );
                        }
                    }
                }
            }

            for msg in self.footer.update(app) {
                match msg {
                    TurnMessages::TogglePlay => {
                        let now = self.current_et.get();
                        commit_pending_builds(&self.world, &self.parts, now);
                        self.recompute_run_until();
                        self.paused = !self.paused;
                        if !self.paused {
                            self.footer.resumed();
                        }
                    }
                    TurnMessages::SpeedUp => self.sim_speed.speed_up(),
                    TurnMessages::SlowDown => self.sim_speed.slow_down(),
                }
            }
        }

        if !self.paused {
            let dt = (1.0 / 60.0_f64) * self.sim_speed.get_rate(); // TODO: Expose delta_seconds
            let mut t = self.current_et.get() + EphemerisTime::from_secs(dt);

            if let Some(stop) = self.run_until {
                if t >= stop {
                    t = stop; // land exactly on the boundary
                    self.paused = true
                }
            }
            self.current_et.set(t);

            if self.paused {
                // Save what stopped us so that we can display it to the player
                self.footer.stopped_at(t);

                for event in self.event_queue.pop_due(t) {
                    self.handle_event(event, app);
                }
                self.complete_due_jobs(t, app);
                self.recompute_run_until();
            }

            if !self.game_over_ui.is_shown() {
                if let Some((station, cause)) = crew_death(&self.world, self.current_et.get()) {
                    let now = self.current_et.get();
                    commit_station(&self.world, station, now);
                    self.world.get::<&mut Station>(station).unwrap().num_crew = 0;
                    self.paused = true;
                    self.run_until = None;

                    let name = self
                        .world
                        .get::<&Named>(station)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    self.game_over_ui.show(&name, cause, now, app);
                }
            }
        }

        // Update GUI stuff
        self.controls_enabled.set(self.paused);

        orbit_system(&mut self.world, self.current_et.get());
        docked_position_system(&mut self.world);
        landed_system(&mut self.world);
        self.select_system();
        self.camera_update(app);
        if !modal_open {
            self.hovered = None;
            self.control(app);
            self.mouse_hover_system(app, false);
            self.tile_select_system(app);
            self.mouse_hover_system(app, true);
        }
        self.sync_selected_tile(app);
        self.line_path_system(app);
        self.sync_models(app);
        let marks = self.build_marks();
        self.footer.set_marks(marks);
        self.sync_panel(app);

        // Delete anything we want deleted
        app.renderer.flush_deletion_queue();
    }

    /// Render the scene to the screen when time allows
    fn render(&mut self, app: &App) {
        // Set everything up
        let aspect = app.window_size.x as f32 / app.window_size.y as f32;
        if (self.camera_3d.inner.aspect_ratio() - aspect).abs() > 1e-6 {
            self.camera_3d.inner.set_aspect_ratio(aspect);
        }

        self.directional_light.light_dir =
            -nalgebra_glm::convert::<DVec3, Vec3>(self.camera_3d.world_pos);
        app.renderer.set_camera(self.camera_3d.inner);
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        app.renderer.set_font(font);

        // Draw the 3D stuff
        app.renderer.set_color(vec4(0.01, 0.01, 0.01, 1.0));
        app.renderer.clear();
        self.starbox.draw(app);
        self.render_dots(app);
        app.renderer.directional_light_system(
            &mut self.directional_light,
            &mut self.world,
            &self.bvh,
        );
        app.renderer.render_3d_models_system(
            &mut self.world,
            &self.directional_light,
            &self.bvh,
            Some(&self.camera_3d),
            false,
        );
        app.renderer
            .render_3d_line_paths(&self.world, Some(&self.camera_3d));

        // Draw the 2D stuff
        // Draw selected reticle
        if let Some(entity) = self.selection.selected_entity() {
            if !self.selection.is_animating(app.seconds as f64)
                && self.world.get::<&Craft>(entity).is_ok()
            {
                let reticle_texture = app.renderer.get_texture_id_from_name("reticle").unwrap();
                const WIDTH: f32 = 16.0;
                app.renderer.copy_texture(
                    Rectangle::new(
                        (app.window_size.x as f32 - WIDTH) * 0.5,
                        (app.window_size.y as f32 - WIDTH) * 0.5,
                        WIDTH,
                        WIDTH,
                    ),
                    reticle_texture,
                    Rectangle::new(0.0, 0.0, WIDTH, WIDTH),
                    &vec4(1.0, 1.0, 1.0, 1.0),
                );
            }
        }

        // Draw hovered reticle
        if let (Some(hovered), Some(selected)) = (self.hovered, self.selection.selected_entity()) {
            if hovered != selected {
                let hovered_world_pos = self.world.get::<&WorldPosition>(hovered).unwrap().pos;
                let named = self.world.get::<&Named>(hovered).unwrap();

                let radius = self
                    .world
                    .get::<&Body>(hovered)
                    .map(|b| b.body_radius)
                    .unwrap_or(0.0);

                let relative_pos = hovered_world_pos - self.camera_3d.world_pos;
                match self.world_to_screen(relative_pos, app) {
                    Some(screen_pos)
                        if self.apparent_radius_px(radius, relative_pos.norm(), app) < 2.0
                            && !self.is_occluded(hovered, relative_pos) =>
                    {
                        let width = 16.0;

                        let reticle_texture =
                            app.renderer.get_texture_id_from_name("reticle").unwrap();
                        app.renderer.copy_texture(
                            Rectangle::new(
                                screen_pos.x - width * 0.5,
                                screen_pos.y - width * 0.5,
                                width,
                                width,
                            ),
                            reticle_texture,
                            Rectangle::new(0.0, 0.0, 16.0, 16.0),
                            &vec4(1.0, 1.0, 1.0, 1.0),
                        );
                        app.renderer
                            .draw_text(screen_pos + vec2(8.0, 8.0), &named.name);
                    }
                    _ => {}
                };
            }
        }

        let (view_matrix, proj_matrix) = self.camera_3d.inner.view_proj_matrices();
        for (selected, index, line_path) in [&self.selected_tile, &self.hovered_tile]
            .into_iter()
            .flatten()
        {
            let world_pos = self.world.get::<&WorldPosition>(*selected).unwrap().pos;
            let relative_pos = world_pos - self.camera_3d.world_pos;

            let d = relative_pos.norm();
            let r = self.world.get::<&Body>(*selected).unwrap().body_radius;
            let to_camera = -relative_pos / d;

            let tile_dir = self
                .world
                .get::<&TileMap>(*selected)
                .unwrap()
                .tile_offset(*index as u32, 1.0);

            if tile_dir.dot(&to_camera) <= r / d {
                continue;
            }

            app.renderer.draw_line_path_at(
                line_path,
                nalgebra_glm::convert(relative_pos),
                view_matrix,
                proj_matrix,
            );
        }

        // Draw GUI
        self.gui.render(app);
        self.footer.render(app);
        self.fabricator_ui.render(app);
        self.maneuver_ui.render(app);
        self.transfer_ui.render(app);
        self.game_over_ui.render(app);
    }
}

impl Gameplay {
    /// Constructs a new Gameplay struct with everything setup
    /// TODO: Most of this stuff will need to be moved to the init scene. Remind me to make an issue for this!
    pub fn new(app: &App) -> Self {
        let mut world = World::new();

        // Add programs to the renderer
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/3d.vert"),
                include_str!("../shaders/3d.frag"),
            )
            .unwrap(),
            Some("3d"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/2d.vert"),
                include_str!("../shaders/2d.frag"),
            )
            .unwrap(),
            Some("2d"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/shadow.vert"),
                include_str!("../shaders/shadow.frag"),
            )
            .unwrap(),
            Some("shadow"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/2d.vert"),
                include_str!("../shaders/solid-color.frag"),
            )
            .unwrap(),
            Some("2d-solid"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/3d.vert"),
                include_str!("../shaders/solid-color.frag"),
            )
            .unwrap(),
            Some("3d-solid"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/line.vert"),
                include_str!("../shaders/line.frag"),
            )
            .unwrap(),
            Some("line"),
        );
        app.renderer.add_program(
            create_program(
                include_str!("../shaders/starbox.vert"),
                include_str!("../shaders/starbox.frag"),
            )
            .unwrap(),
            Some("starbox"),
        );

        // Setup the mesh manager
        app.renderer
            .add_mesh_from_obj(QUAD_XY_DATA, Some("quad-xy"));
        app.renderer.add_mesh_from_obj(UV_DATA, Some("uv"));
        app.renderer.add_mesh_from_obj(CONE_DATA, Some("cone"));
        app.renderer.add_mesh_from_obj(CUBE_DATA, Some("cube"));

        let ico_20 = icosphere::generate(0); // 20-face icosphere for dwarf bodies
        let ico_80 = icosphere::generate(1); // 80-face icosphere for mars-like sub-earths
        let ico_320 = icosphere::generate(2); // 320-face icosphere for large rocky bodies
        app.renderer.add_mesh_from_verts(
            ico_20.indices.clone(),
            vec![&ico_20.positions, &ico_20.normals, &ico_20.uvs],
            Some("ico-20"),
        );
        app.renderer.add_mesh_from_verts(
            ico_80.indices.clone(),
            vec![&ico_80.positions, &ico_80.normals, &ico_80.uvs],
            Some("ico-80"),
        );
        app.renderer.add_mesh_from_verts(
            ico_320.indices.clone(),
            vec![&ico_320.positions, &ico_320.normals, &ico_320.uvs],
            Some("ico-320"),
        );
        let tile_sets = TileSets {
            dwarf: ico_20.tile_tris,
            sub: ico_80.tile_tris,
            large: ico_320.tile_tris,
        };

        for (i, name) in ["triangle", "square", "pentagon", "hexagon"]
            .iter()
            .enumerate()
        {
            let sides = i + 3;
            let (indices, pos, normals, uvs) = polygon::ngon_mesh(sides as u32);
            app.renderer
                .add_mesh_from_verts(indices, vec![&pos, &normals, &uvs], Some(name));
        }

        for (i, name) in [
            "triangle-outline",
            "square-outline",
            "pentagon-outline",
            "hexagon-outline",
            "septagon-outline",
            "octagon-outline",
        ]
        .iter()
        .enumerate()
        {
            let sides = i + 3;
            let (indices, pos, normals, uvs) = polygon::ngon_ring_mesh(sides as u32, 0.875);
            app.renderer
                .add_mesh_from_verts(indices, vec![&pos, &normals, &uvs], Some(name));
        }

        // Setup the texture manager
        app.renderer
            .add_texture_from_png("res/sun.png", Some("sun"));
        app.renderer
            .add_texture_from_png("res/venus.png", Some("venus"));
        app.renderer
            .add_texture_from_png("res/earth.png", Some("earth"));
        app.renderer
            .add_texture_from_png("res/moon.png", Some("moon"));
        app.renderer
            .add_texture_from_png("res/jupiter.png", Some("jupiter"));
        app.renderer
            .add_texture_from_png("res/europa.png", Some("europa"));
        app.renderer
            .add_texture_from_png("res/uranus.png", Some("uranus"));
        app.renderer
            .add_texture_from_png("res/next-turn.png", Some("next-turn"));
        app.renderer
            .add_texture_from_png("res/next-turn-hover.png", Some("next-turn-hover"));
        app.renderer
            .add_texture_from_png("res/reticle.png", Some("reticle"));

        // Setup the font manager
        app.renderer
            .add_font("res/Consolas.ttf", "font", 15, sdl2::ttf::FontStyle::NORMAL);
        app.renderer.add_font(
            "res/Consolas.ttf",
            "font-small-bold",
            16,
            sdl2::ttf::FontStyle::BOLD,
        );
        app.renderer.add_font(
            "res/Consolas.ttf",
            "font-small-italic",
            16,
            sdl2::ttf::FontStyle::ITALIC,
        );
        app.renderer.add_font(
            "res/Consolas.ttf",
            "font-big",
            21,
            sdl2::ttf::FontStyle::BOLD,
        );

        let mut bvh = BVH::<Entity>::new();

        let sun_entity = spawn_body(
            Body {
                category: Category::Star,
                body_radius: 110.0,
                rotation_period_hours: 0.0,
                rotation: 0.0,
                atmos_pressure: 1000000.0,
                temperature: 5778.0,
                core_mass_fraction: 0.0,
                magnetic_field: true,
                density: 1.0,
                mu: SUN_MU,
            },
            State::circular(0.1, EphemerisTime::new(rand::random()), 1.0),
            SceneObject { bvh_node_id: None },
            Named {
                name: String::from("The Sun"),
            },
            None,
            &tile_sets,
            &mut world,
            &app.renderer,
            &mut bvh,
        );

        let mut bodies = vec![sun_entity];
        let mut crafts = vec![];
        let buildings = vec![];

        let (_lexicon, _node_count) = Lexicon::create("res/names.txt", "res/names.lex");
        let lexicon = Lexicon::read("res/names.lex");

        let parts = PartRegistry::load_from_dir("res/parts");

        let mut station_parent = None;
        let (planets, starter) = solar_system_gen::generate();
        for (i, system) in planets.into_iter().enumerate() {
            let name = lexicon.generate_word(7);
            println!("Planet: {}", name);

            let planet_entity = spawn_body(
                system.planet.0,
                system.planet.1,
                SceneObject { bvh_node_id: None },
                Named { name },
                Some(Parent { id: sun_entity }),
                &tile_sets,
                &mut world,
                &app.renderer,
                &mut bvh,
            );

            bodies.push(planet_entity);
            if i == starter {
                station_parent = Some(planet_entity)
            }

            for moon in &system.moons {
                let name = lexicon.generate_word(10);
                println!("Moon: {}", name);
                let moon_entity = spawn_body(
                    moon.0,
                    moon.1,
                    SceneObject { bvh_node_id: None },
                    Named { name },
                    Some(Parent { id: planet_entity }),
                    &tile_sets,
                    &mut world,
                    &app.renderer,
                    &mut bvh,
                );
                bodies.push(moon_entity);
            }
        }

        let station_parent = station_parent.expect("generator returned no station host");
        let parent_mu = world.get::<&Body>(station_parent).unwrap().mu;
        let parent_body_radius = world.get::<&Body>(station_parent).unwrap().body_radius;
        let parent_pos = world.get::<&WorldPosition>(station_parent).unwrap().pos;

        let station_payload = parts
            .all()
            .find(|p| p.id == "station_core")
            .unwrap()
            .instantiate_craft();

        let station = spawn_craft(
            station_payload,
            SceneObject { bvh_node_id: None },
            Named {
                name: String::from("Station"),
            },
            Parent { id: station_parent },
            &mut world,
            &app.renderer,
            &mut bvh,
        );

        let station_state = State::from_kepler(
            parent_body_radius * 16.0,
            0.2,
            0.0,
            1.5,
            0.15,
            0.15,
            EphemerisTime::new(0),
            parent_mu,
        );
        world.insert_one(station, station_state).unwrap();

        let vertices: Vec<f32> = station_state
            .generate_orbit_vertices(8192, parent_mu, None)
            .unwrap()
            .iter()
            .flat_map(|v| v.iter().map(|x| *x as f32))
            .collect();

        replace_line_path(
            &mut world,
            &app.renderer,
            station,
            Some((
                WorldPosition { pos: parent_pos },
                Parent { id: station_parent },
                LinePathComponent::new(vertices),
                AssociatedEntity { associate: station },
            )),
        );

        let mut starting_inventory = PartInventory {
            parts: HashMap::new(),
        };

        starting_inventory.add(id_hash("ilmenite"), 8);

        world
            .insert(
                station,
                (
                    Station { num_crew: 2 },
                    PortHost {
                        dock_gen: 0,
                        ports: 8,
                    },
                    starting_inventory,
                ),
            )
            .unwrap();
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 0,
            },
            ResourceStore {
                resource: Resource::Energy,
                amount: 4.32e8,
                capacity: 1.8e9,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 1,
            },
            SolarPanel { rated_w: 100_000.0 },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 2,
            },
            ResourceStore {
                resource: Resource::Water,
                amount: 3800.0,
                capacity: 3800.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 3,
            },
            ResourceStore {
                resource: Resource::Oxygen,
                amount: 10.0,
                capacity: 600.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 4,
            },
            ResourceStore {
                resource: Resource::Hydrogen,
                amount: 0.0,
                capacity: 100.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 5,
            },
            Factory {
                current_job: None,
                pending_job: None,
                power_watts: 5000.0,
                enabled: false,
                reserved_port: None,
            },
            Parent { id: station_parent },
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 6,
            },
            Electrolyzer {
                enabled: false,
                power_watts: 5_000.0,
                joules_per_kg_water: 2.52e7,
            },
            Parent { id: station_parent },
        ));
        crafts.push(station);

        let mut selection = SelectionState::new(crafts, bodies, buildings);
        selection.set_selected(station, app.seconds as f64 - 1.0);

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        app.renderer.set_font(font);

        let event_queue = EventQueue::new();

        let mut retval = Self {
            world,
            camera_3d: high_precision::Camera {
                world_pos: vec3(1.0, 1.0, 1.0),
                inner: Camera::new(
                    vec3(1.0, 0.0, 1.0),
                    vec3(0.0, 0.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    ProjectionKind::Perspective {
                        fov_rad: 67.0f32.to_radians(),
                        far: 10000000.0,
                    },
                    4.0 / 3.0,
                ),
            },
            bvh,
            directional_light: DirectionalLightSource::new(
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

            selection,
            hovered: None,
            selected_tile: None,
            clicked_tile_key: None,
            hovered_tile: None,

            parts,

            phi: 2.5,
            theta: -PI / 4.0,
            distance: 64.0,
            prev_tab_state: false,

            footer: Footer::new(),
            gui: Anchor::new(Box::new(container![]), AnchorPoint::TopRight),
            gui_built_for: None,
            gui_bindings: vec![],
            fabricator_ui: FabricatorUi::new(),
            maneuver_ui: ManeuverModal::new(app),
            transfer_ui: TransferUi::new(),
            game_over_ui: GameOverUi::new(),

            controls_enabled: Rc::new(Cell::new(false)),

            current_et: Rc::new(Cell::new(EphemerisTime::epoch())),
            event_queue,
            paused: true,
            sim_speed: SimSpeed::new(),
            run_until: None,

            starbox: Starbox::new(9000, vec3(1.0, 2.0, 4.0), 0.4),
        };

        retval.sync_panel(app);

        retval
    }

    fn recompute_run_until(&mut self) {
        let now = self.current_et.get();
        self.schedule_events();
        let next_event = self.event_queue.events.keys().next().copied();
        let next_limit = self.next_station_limit(now);
        let next_job_complete = self.next_job_completion(now);

        self.run_until = [next_event, next_limit, next_job_complete]
            .into_iter()
            .flatten()
            .min()
    }

    fn next_job_completion(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        self.world
            .query::<&Factory>()
            .iter()
            .filter_map(|(_, f)| f.current_job.as_ref()?.completion_et(f, now))
            .min()
    }

    /// Changes various game state based on user mouse and keyboard input
    fn control(&mut self, app: &App) {
        let curr_tab_state = app.keys[Scancode::Tab as usize];
        let curr_shift_state =
            app.keys[Scancode::LShift as usize] || app.keys[Scancode::RShift as usize];
        if curr_tab_state && !self.prev_tab_state {
            if curr_shift_state {
                self.selection.prev(app.seconds as f64);
            } else {
                self.selection.next(app.seconds as f64);
            }
        }
        self.prev_tab_state = curr_tab_state;

        let body_radius = self.get_selected_body_radius().unwrap_or(0.0);
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

    fn gui_structure_key(&self) -> Option<(Entity, u32, u64, u64)> {
        let sel = self.selection.selected_entity()?;
        let gen = self
            .world
            .get::<&PortHost>(sel)
            .map(|s| s.dock_gen)
            .unwrap_or(0);

        Some((
            sel,
            gen,
            self.event_queue.version(),
            panel_structure_bits(&self.world),
        ))
    }

    fn undock(&mut self, craft: Entity, app: &App) {
        const SEPARATION_DV: f64 = 0.1 / METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR;

        let now = self.current_et.get();

        let Ok(host) = self.world.get::<&Docking>(craft).map(|d| d.host) else {
            return; // not docked?!
        };
        let parent = self.world.get::<&Parent>(craft).unwrap().id;
        let parent_mu = self.world.get::<&Body>(parent).unwrap().mu;
        let parent_world_pos = self.world.get::<&WorldPosition>(parent).unwrap().pos;

        // Set the new state of the craft to be the host state + a little radial boost
        let Ok(host_state) = self.world.get::<&State>(host).map(|s| *s) else {
            return; // host wasn't orbiting
        };
        let Ok(mut new_state) = host_state.propagate(now, parent_mu) else {
            return;
        };
        new_state.v += new_state.r.normalize() * SEPARATION_DV;

        // Commit resource flows now
        commit_station(&self.world, host, now);
        commit_station(&self.world, craft, now);

        let vertices: Vec<f32> = new_state
            .generate_orbit_vertices(8192, parent_mu, None)
            .unwrap()
            .iter()
            .flat_map(|v| v.iter().map(|x| *x as f32))
            .collect();

        replace_line_path(
            &mut self.world,
            &app.renderer,
            craft,
            Some((
                WorldPosition {
                    pos: parent_world_pos,
                },
                Parent { id: parent },
                LinePathComponent::new(vertices),
                AssociatedEntity { associate: craft },
            )),
        );

        self.world.remove_one::<Docking>(craft).ok();
        self.world.insert_one(craft, new_state).unwrap();

        if let Ok(mut ph) = self.world.get::<&mut PortHost>(craft) {
            ph.dock_gen += 1;
        }
        if let Ok(mut ph) = self.world.get::<&mut PortHost>(host) {
            ph.dock_gen += 1;
        }
    }

    fn schedule_events(&mut self) {
        let crafts_with_commands: Vec<(Entity, Command)> = self
            .world
            .query::<(&mut Craft,)>()
            .iter()
            .filter_map(|(entity, (craft,))| {
                if craft.command.is_some() && !craft.command_scheduled {
                    craft.command_scheduled = true;
                    craft.command.as_ref().map(|cmd| (entity, cmd.clone()))
                } else {
                    None
                }
            })
            .collect();

        for (entity, command) in crafts_with_commands {
            match command {
                Command::Transfer { to, plan, .. } => {
                    let departure_time = plan.transfer_state.t;
                    let arrival_time = plan.flyby_state.t;
                    let circ_time = plan.circ_state.t;

                    println!("departure_time: {}", departure_time.as_calendar());
                    println!("arrival_time.t: {}", arrival_time.as_calendar());
                    println!("circ_time.t: {}", circ_time.as_calendar());

                    assert!(departure_time < arrival_time);
                    assert!(arrival_time < circ_time);

                    let sois = command.transition_schedule();
                    self.event_queue.push(
                        arrival_time,
                        Event::SoiChange {
                            craft: entity,
                            new_parent: to,
                            new_craft_orbit: plan.flyby_state,
                            new_soi_radius: plan.soi_radius,
                            desc: sois[0].0,
                        },
                    );

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(circ_time, Event::CompleteCommand { craft: entity });
                }
                Command::Flyby { to, from, plan, .. } => {
                    let departure_time = plan.transfer_state.t;
                    let arrival_time = plan.flyby_state.t;
                    let exit_time = plan.exit_state.t;

                    println!("departure_time: {}", departure_time.as_calendar());
                    println!("arrival_time.t: {}", arrival_time.as_calendar());
                    println!("exit_time.t: {}", exit_time.as_calendar());

                    assert!(departure_time < arrival_time);
                    assert!(arrival_time < exit_time);

                    let sois = command.transition_schedule();

                    // Enter SOI event
                    self.event_queue.push(
                        arrival_time,
                        Event::SoiChange {
                            craft: entity,
                            new_parent: to,
                            new_craft_orbit: plan.flyby_state,
                            new_soi_radius: plan.soi_radius,
                            desc: sois[0].0,
                        },
                    );

                    // Exit SOI event
                    self.event_queue.push(
                        exit_time,
                        Event::SoiChange {
                            craft: entity,
                            new_parent: from,
                            new_craft_orbit: plan.exit_state,
                            new_soi_radius: plan.soi_radius,
                            desc: sois[1].0,
                        },
                    );

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(exit_time, Event::CompleteCommand { craft: entity });
                }
                Command::Rendezvous { plan, .. } => {
                    let arrival_time = plan.rendezvous_state.t;

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(arrival_time, Event::CompleteCommand { craft: entity });
                }
                Command::Escape { to, plan, .. } => {
                    let departure_time = plan.escape_burn.t;
                    let arrival_time = plan.exit_state.t;

                    println!("departure_time: {}", departure_time.as_calendar());
                    println!("arrival_time.t: {}", arrival_time.as_calendar());

                    assert!(departure_time < arrival_time);

                    let sois = command.transition_schedule();
                    self.event_queue.push(
                        arrival_time,
                        Event::SoiChange {
                            craft: entity,
                            new_parent: to,
                            new_craft_orbit: plan.exit_state,
                            new_soi_radius: plan.soi_radius,
                            desc: sois[0].0,
                        },
                    );

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(arrival_time, Event::CompleteCommand { craft: entity });
                }
                Command::Launch { plan, .. } => {
                    let launch_time = plan.launch_burn.t;
                    let circ_time = plan.circ_burn.t;

                    println!("launch_time: {}", launch_time.as_calendar());
                    println!("circ_time.t: {}", circ_time.as_calendar());

                    assert!(launch_time < circ_time);

                    self.event_queue
                        .push(launch_time, Event::Launch { craft: entity });

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(circ_time, Event::CompleteCommand { craft: entity });
                }
                Command::Land { plan, .. } => {
                    let deorbit_time = plan.deorbit_burn.t;
                    let land_time = plan.landing_burn.t;

                    println!("deorbit_time: {}", deorbit_time.as_calendar());
                    println!("land_time.t: {}", land_time.as_calendar());

                    assert!(deorbit_time < land_time);

                    for burn in command.burn_schedule() {
                        self.event_queue.push(
                            burn.t(),
                            Event::Burn {
                                craft: entity,
                                new_orbit: burn.new_orbit,
                                soi_radius: burn.soi_radius,
                                dv: burn.dv,
                                desc: burn.desc,
                                purpose: burn.purpose,
                            },
                        )
                    }

                    self.event_queue
                        .push(land_time, Event::Land { craft: entity });
                    self.event_queue
                        .push(land_time, Event::CompleteCommand { craft: entity });
                }
                Command::Dock {
                    with, arrive_et, ..
                } => {
                    self.event_queue.push(
                        arrive_et,
                        Event::Dock {
                            craft: entity,
                            with,
                        },
                    );
                    self.event_queue
                        .push(arrive_et, Event::CompleteCommand { craft: entity });
                }
            }
        }
    }

    fn handle_event(&mut self, event: Event, app: &App) {
        match event {
            Event::SoiChange {
                craft,
                new_parent,
                new_craft_orbit,
                new_soi_radius,
                ..
            } => {
                self.selection.set_selected(craft, app.seconds as f64);

                let new_parent_world_pos =
                    self.world.get::<&WorldPosition>(new_parent).unwrap().pos;
                let new_parent_mu = self.world.get::<&Body>(new_parent).unwrap().mu;

                let vertices: Vec<f32> = new_craft_orbit
                    .generate_orbit_vertices(8192, new_parent_mu, Some(new_soi_radius))
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.iter().map(|x| *x as f32))
                    .collect();

                replace_line_path(
                    &mut self.world,
                    &app.renderer,
                    craft,
                    Some((
                        WorldPosition {
                            pos: new_parent_world_pos, // center the orbit line path about the new parent
                        },
                        Parent { id: new_parent },
                        LinePathComponent::new(vertices),
                        AssociatedEntity { associate: craft },
                    )),
                );
                self.world.remove_one::<State>(craft).ok();
                self.world
                    .insert(craft, (new_craft_orbit, Parent { id: new_parent }))
                    .unwrap();
            }
            Event::Burn {
                craft,
                new_orbit,
                soi_radius,
                dv,
                ..
            } => {
                self.selection.set_selected(craft, app.seconds as f64);

                println!(
                    "Burn firing, r={:?} v={:?} at {}",
                    new_orbit.r,
                    new_orbit.v,
                    self.current_et.get().as_calendar()
                );
                let parent = self.world.get::<&Parent>(craft).unwrap().id;
                let parent_world_pos = self.world.get::<&WorldPosition>(parent).unwrap().pos;
                let parent_mu = { self.world.get::<&Body>(parent).unwrap().mu };

                let vertices: Vec<f32> = new_orbit
                    .generate_orbit_vertices(8192, parent_mu, soi_radius)
                    .unwrap()
                    .iter()
                    .flat_map(|v| v.iter().map(|x| *x as f32))
                    .collect();

                replace_line_path(
                    &mut self.world,
                    &app.renderer,
                    craft,
                    Some((
                        WorldPosition {
                            pos: parent_world_pos,
                        },
                        Parent { id: parent },
                        LinePathComponent::new(vertices),
                        AssociatedEntity { associate: craft },
                    )),
                );
                apply_burn(&self.world, craft, dv, self.current_et.get());
                self.world.remove_one::<State>(craft).ok();
                self.world
                    .insert(craft, (new_orbit, Parent { id: parent }))
                    .unwrap();
            }
            Event::Launch { craft } => {
                self.selection.set_selected(craft, app.seconds as f64);

                println!(
                    "Launch event firing for {:?} at {}",
                    craft,
                    self.current_et.get().as_calendar()
                );
                let parent_id = self.world.get::<&Parent>(craft).unwrap().id;
                commit_station(&self.world, craft, self.current_et.get());
                self.world.remove_one::<Landed>(craft).ok();
                self.world
                    .insert(craft, (Parent { id: parent_id },))
                    .unwrap();
            }
            Event::Land { craft } => {
                self.selection.set_selected(craft, app.seconds as f64);

                let offset = {
                    let craft_state = self.world.get::<&State>(craft).unwrap();
                    let parent_id = self.world.get::<&Parent>(craft).unwrap().id;
                    let parent_body_mu = self.world.get::<&Body>(parent_id).unwrap().mu;
                    craft_state
                        .propagate(self.current_et.get(), parent_body_mu)
                        .unwrap()
                        .r
                };

                self.world.remove_one::<State>(craft).ok();
                replace_line_path(&mut self.world, &app.renderer, craft, None);
                commit_station(&self.world, craft, self.current_et.get());
                self.world.insert_one(craft, Landed { offset }).unwrap();
            }
            Event::Dock { craft, with } => {
                let Some((own_port, host_port)) = allocate_ports(&self.world, craft, with) else {
                    // port got taken while we were in transit. Just stay in orbit.
                    return;
                };

                self.selection.set_selected(craft, app.seconds as f64);

                {
                    let mut docks = self.world.get::<&mut PortHost>(craft).unwrap();
                    docks.dock_gen += 1;
                }

                self.world.remove_one::<State>(craft).ok();
                replace_line_path(&mut self.world, &app.renderer, craft, None);
                commit_station(&self.world, craft, self.current_et.get());
                self.world
                    .insert_one(
                        craft,
                        Docking {
                            host: with,
                            host_port,
                            own_port,
                        },
                    )
                    .unwrap();
            }
            Event::CompleteCommand { craft } => {
                let mut craft = self.world.get::<&mut Craft>(craft).unwrap();
                craft.command = None;
                craft.command_scheduled = false;
            }
            Event::FactoryComplete { .. } => {
                // Nothing to do here, factory completes are handled elsewhere.
            }
        }
    }

    fn commit_station(&self) {
        for (station, _) in self.world.query::<&PortHost>().iter() {
            commit_station(&self.world, station, self.current_et.get());
        }
    }

    fn complete_due_jobs(&mut self, now: EphemerisTime, app: &App) {
        // TODO: Split up into sim + render, move to resp. modules
        let done: Vec<(Entity, u64)> = self
            .world
            .query::<&Factory>()
            .iter()
            .filter_map(|(e, f)| {
                let job = f.current_job.as_ref()?;
                (job.energy_at(f, now) >= job.energy_total - f.power_watts)
                    .then_some((e, job.part_id))
            })
            .collect();

        for (fab, part_id) in done {
            self.selection.set_selected(fab, app.seconds as f64);

            let parent = self.world.get::<&Parent>(fab).unwrap().id;
            let def = self.parts.get(part_id).unwrap().clone();

            self.commit_station();

            // Add any byproducts
            for (r, amt) in &def.byproducts {
                add_resource(&self.world, parent, *r, *amt, self.current_et.get());
            }

            // For now just eject the stage
            if def.cost.ports_required > 0 {
                let host_port = {
                    self.world
                        .get::<&Factory>(fab)
                        .unwrap()
                        .reserved_port
                        .unwrap()
                };
                self.deliver_craft(parent, &def, host_port, app);
            } else {
                let mut part_inventory = self.world.get::<&mut PartInventory>(parent).unwrap();
                part_inventory.add(part_id, 1);
            }

            // clear job so that factory becomes idle
            if let Ok(mut f) = self.world.get::<&mut Factory>(fab) {
                f.current_job = None;
                f.reserved_port = None
            }
        }
    }

    fn deliver_craft(&mut self, station: Entity, def: &PartDef, host_port: u32, app: &App) {
        // TODO: Split up into sim + render, move to resp. modules
        let now = self.current_et.get();
        let parent = *self.world.get::<&Parent>(station).unwrap();

        let craft = spawn_craft(
            def.instantiate_craft(),
            SceneObject { bvh_node_id: None },
            Named {
                name: def.name.clone(),
            },
            parent,
            &mut self.world,
            &app.renderer,
            &mut self.bvh,
        );
        self.world
            .insert_one(
                craft,
                PortHost {
                    dock_gen: 0,
                    ports: def.ports,
                },
            )
            .unwrap();

        for (port, spec) in def.modules.iter().enumerate() {
            let docking = Docking {
                host: craft,
                host_port: port as u32,
                own_port: 0 as u32, // TODO: This will work for modules now, but maybe break if modules get multiple docking ports
            };
            let parent = Parent { id: craft };
            match *spec {
                ModuleSpec::Store {
                    resource,
                    amount,
                    capacity,
                } => self.world.spawn((
                    docking,
                    ResourceStore {
                        resource,
                        amount,
                        capacity,
                        amount_et: now,
                    },
                    parent,
                )),
                ModuleSpec::Miner {
                    power_watts,
                    kg_per_s,
                } => self.world.spawn((
                    docking,
                    Miner {
                        enabled: false,
                        kg_per_s,
                        power_watts,
                    },
                    parent,
                )),
            };
        }

        let own_port = next_free_port(&self.world, craft).expect("gotta have a port babey");
        self.world
            .insert_one(
                craft,
                Docking {
                    host: station,
                    host_port,
                    own_port,
                },
            )
            .unwrap();

        self.selection.crafts.push(craft);
    }

    fn sync_models(&mut self, app: &App) {
        for (_entity, (world_pos, model, scene_obj)) in
            self.world
                .query_mut::<(&WorldPosition, &mut ModelComponent, &SceneObject)>()
        {
            let new_pos: Vec3 = nalgebra_glm::convert(world_pos.pos - self.camera_3d.world_pos);
            model.set_position(new_pos);
            self.bvh.move_obj(
                scene_obj.bvh_node_id.unwrap(),
                &app.renderer.get_model_aabb(model),
                &vec3(0.0f32, 0.0, 0.0),
            );
        }
    }

    fn build_marks(&self) -> Vec<TimelineMark> {
        // Add hard events from the event queue
        let mut marks: Vec<TimelineMark> = self
            .event_queue
            .events
            .iter()
            .flat_map(|(et, events)| {
                let t = *et;
                events.iter().filter_map(move |event| {
                    let (subject, detail) = self.craft_name_from_event(event);
                    Some(TimelineMark {
                        t,
                        kind: MarkKind::from_event(event)?,
                        subject,
                        detail,
                    })
                })
            })
            .collect();

        // Add pending projected factory completion events
        for (fab, (_, f)) in self.world.query::<(&Docking, &Factory)>().iter() {
            let (t, part_id) = if let Some(part_id) = f.pending_job {
                (
                    projected_completion(&self.world, fab, &self.parts, self.current_et.get())
                        .unwrap(),
                    part_id,
                )
            } else if let Some(current_job) = &f.current_job {
                let Some(completion_et) = current_job.completion_et(f, self.current_et.get())
                else {
                    continue;
                };
                (completion_et, current_job.part_id)
            } else {
                continue;
            };

            let (subject, detail) = self.craft_name_from_event(&Event::FactoryComplete {
                craft: fab,
                part_id,
            });

            marks.push(TimelineMark {
                t,
                kind: MarkKind::FactoryComplete,
                subject,
                detail,
            });
        }

        // Add projected reservoir limit events, Depleted and Filled
        for (entity, (_, named)) in self.world.query::<(&PortHost, &Named)>().iter() {
            for (et, resource, rate) in next_reservoir_limits(
                &self.world,
                entity,
                &self.parts,
                self.current_et.get(),
                true,
            ) {
                if rate < 0.0 {
                    marks.push(TimelineMark {
                        t: et,
                        kind: MarkKind::Critical,
                        subject: named.name.clone(),
                        detail: format!("{} Depleted", resource.long_name()),
                    })
                } else {
                    marks.push(TimelineMark {
                        t: et,
                        kind: MarkKind::Good,
                        subject: named.name.clone(),
                        detail: format!("{} Filled", resource.long_name()),
                    })
                }
            }
        }

        // Add projected mission burns (burns, SOI crossings, etc)
        for (_, (craft, named)) in self.world.query::<(&Craft, &Named)>().iter() {
            let Some(command) = &craft.command else {
                continue;
            };
            if craft.command_scheduled {
                continue; // already in the event queue, don't re-add it
            }
            for burn in command.burn_schedule() {
                marks.push(TimelineMark {
                    t: burn.t(),
                    kind: burn.purpose.into(),
                    subject: named.name.clone(),
                    detail: burn.desc.to_string(),
                });
            }
            for (label, et) in command.transition_schedule() {
                marks.push(TimelineMark {
                    t: et,
                    kind: MarkKind::SoiChange,
                    subject: named.name.clone(),
                    detail: label.to_string(),
                });
            }
        }

        marks.sort_by_key(|m| m.t);
        marks
    }

    fn next_station_limit(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        let mut limits = vec![];
        for (entity, _) in self.world.query::<&PortHost>().iter() {
            limits.extend(next_reservoir_limits(
                &self.world,
                entity,
                &self.parts,
                now,
                true,
            ));
        }

        limits.into_iter().map(|(et, _, _)| et).min()
    }

    fn craft_name_from_event(&self, event: &Event) -> (String, String) {
        match event {
            Event::SoiChange { craft, desc, .. } | Event::Burn { craft, desc, .. } => {
                let named = self.world.get::<&Named>(*craft).unwrap();
                (named.name.clone(), desc.to_string())
            }

            Event::Launch { craft } | Event::Land { craft } | Event::Dock { craft, .. } => {
                let named = self.world.get::<&Named>(*craft).unwrap();
                (named.name.clone(), "???".to_string())
            }

            Event::FactoryComplete { craft, part_id } => {
                let parent = self.world.get::<&Parent>(*craft).unwrap().id;
                let named = self.world.get::<&Named>(parent).unwrap();
                let part_def = self.parts.get(*part_id).map_or("???", |p| p.name.as_str());
                (named.name.clone(), part_def.to_string())
            }

            // No real craft name
            Event::CompleteCommand { .. } => (String::from(""), String::from("")),
        }
    }

    /// Sets the selected position for the camera to orbit about based on the selected entity
    fn select_system(&mut self) {
        let Some(selected_entity) = self.selection.selected_entity() else {
            return;
        };

        // Buildings keep the camera centered on their planet, not on themselves
        let focus = if self.world.get::<&SurfaceTile>(selected_entity).is_ok() {
            self.world
                .get::<&Parent>(selected_entity)
                .map(|p| p.id)
                .unwrap_or(selected_entity)
        } else {
            selected_entity
        };

        if let Ok(world_pos) = self.world.get::<&WorldPosition>(focus) {
            self.selection.selected_pos = world_pos.pos
        }
    }

    /// Sets the selected entity based on a per-tile check on the already selected entity
    fn tile_select_system(&mut self, app: &App) {
        let pick = self.pick_hovered_tile(app);

        // If the tile is hovered, and selected, make the tile's occupant hovered and selected
        if let Some((entity, tile_index, _)) = &pick {
            let occupant = {
                let tile_map = self.world.get::<&TileMap>(*entity).unwrap();
                tile_map.occupant(*tile_index as u32)
            };

            if let Some(occupant) = occupant {
                self.hovered = Some(occupant)
            }

            if app.mouse_left_clicked && !app.is_click_consumed() {
                self.clicked_tile_key = Some((*entity, *tile_index));

                if let Some(occupant) = occupant {
                    self.selection.set_selected(occupant, app.seconds as f64);
                    app.consume_click();
                }
            }
        }

        // rebuild the line path component if the hovered tile has changed
        Self::sync_tile_path(&mut self.hovered_tile, pick, 0.45, &app.renderer);
    }

    /// Returns the selected entity, the tile index for that entity, and the tile vertices for a tile if it's
    /// hovered over, otherwise None
    fn pick_hovered_tile(&self, app: &App) -> Option<(Entity, usize, Vec<f32>)> {
        if self.hovered.is_some() {
            return None; // some other hover system already wrote to hover
        }

        let Some(selected_entity) = self.selection.selected_entity() else {
            return None; // If nothing is selected, just return
        };

        let body_entity = if self.world.get::<&Body>(selected_entity).is_ok() {
            selected_entity
        } else if let Ok(parent) = self.world.get::<&Parent>(selected_entity) {
            parent.id
        } else {
            return None;
        };

        // Check if the selected is a body
        let mut q = match self
            .world
            .query_one::<(&Body, &WorldPosition, &TileMap)>(body_entity)
        {
            Ok(q) => q,
            Err(_) => return None,
        };
        let (body, pos, tile_map) = q.get()?;

        // Exclude gaseous planets since they dont have tiles
        if body.gaseous() {
            return None;
        }

        // Check if the mouse is hovering over the planet
        let relative_pos = pos.pos - self.camera_3d.world_pos;
        let center = nalgebra_glm::convert(relative_pos);
        let body_sphere = Sphere {
            center,
            radius: body.body_radius as f32,
        };
        let mouse_ray = self.camera_3d.inner.get_ray(
            app.mouse_pos.x,
            app.mouse_pos.y,
            app.window_size.x as f32,
            app.window_size.y as f32,
        );
        let Some(_) = body_sphere.raycast(&mouse_ray) else {
            // not hovering over the planet
            return None;
        };

        // Go through each tile and figure out which one is closest to the ray intersection point
        let r = body.body_radius as f32 * 1.002;
        let local_origin = (mouse_ray.origin() - center) / r;
        let local_ray = Ray::new(local_origin, mouse_ray.dir());

        let (i, _t) = tile_map
            .tris
            .iter()
            .enumerate()
            .filter_map(|(i, tri)| tri.raycast(&local_ray).map(|t| (i, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;

        let vertices = self.tile_outline_vertices(body_entity, i)?;

        Some((body_entity, i, vertices))
    }

    /// Sets the hovered and selected entities for bodies and craft based on a coarse, spherical metric
    fn mouse_hover_system(&mut self, app: &App, bodies: bool) {
        if self.hovered.is_some() {
            return; // some other hover system already wrote to hover
        }

        let mouse_pos = app.mouse_pos;

        for (entity, (world_pos, _model)) in self
            .world
            .query::<hecs::Without<(&WorldPosition, &ModelComponent), &SurfaceTile>>()
            .iter()
        {
            let body = self.world.get::<&Body>(entity);
            if body.is_ok() != bodies {
                continue;
            }
            let relative_pos = world_pos.pos - self.camera_3d.world_pos;
            let screen_pos = self.world_to_screen(relative_pos, app);
            if screen_pos.is_none() {
                continue;
            }

            let radius = body.map(|b| b.body_radius).unwrap_or(0.0);

            let screen_pos = screen_pos.unwrap();
            let l1_dist = nalgebra_glm::l2_norm(&(screen_pos - mouse_pos));
            if (l1_dist as f64)
                < self
                    .apparent_radius_px(radius, relative_pos.norm(), app)
                    .max(16.0)
            {
                self.hovered = Some(entity);
                if app.mouse_left_clicked && !app.is_click_consumed() {
                    self.selection.set_selected(entity, app.seconds as f64);
                    app.consume_click();
                }
                break;
            }
        }
    }

    fn line_path_system(&mut self, app: &App) {
        // Extract out the world positions
        let mut pos_map = HashMap::new();
        for (entity, world_pos) in self.world.query::<&WorldPosition>().iter() {
            pos_map.insert(entity, world_pos.pos);
        }

        // Find which body the camera is closest to, and how close
        let mut closest_body: Option<Entity> = None;
        let mut closest_dist = f64::INFINITY;
        for (entity, (world_pos, _body)) in self.world.query::<(&WorldPosition, &Body)>().iter() {
            let dist = (world_pos.pos - self.camera_3d.world_pos).norm();
            if dist < closest_dist {
                closest_dist = dist;
                closest_body = Some(entity);
            }
        }
        let closest_body =
            get_ancestor(&self.world, closest_body.unwrap()).unwrap_or(self.selection.bodies[0]);
        let closest_planet = get_ancestor(&self.world, closest_body).unwrap_or(closest_body);
        let closest_planet_soi = {
            let closest_planet_body = self.world.get::<&Body>(closest_planet).unwrap();
            let closest_planet_orb = self.world.get::<&State>(closest_planet).unwrap();
            let sun_body = self.world.get::<&Body>(self.selection.bodies[0]).unwrap();
            sphere_of_influence(
                closest_planet_orb.semi_major_axis(SUN_MU),
                closest_planet_body.mass(),
                sun_body.mass(),
            )
        };

        // Get the associated craft, if it exists
        let mut assoc_entity_map = HashMap::new();
        for (entity, _line) in self.world.query::<&LinePathComponent>().iter() {
            assoc_entity_map.insert(
                entity,
                self.world
                    .get::<&AssociatedEntity>(entity)
                    .map_or(Entity::DANGLING, |x| x.associate),
            );
        }

        let mut mu_map = HashMap::new();
        for (entity, (_line, parent)) in self.world.query::<(&LinePathComponent, &Parent)>().iter()
        {
            let parent_entity = parent.id;
            let parent_body_mu = self.world.get::<&Body>(parent_entity).unwrap().mu;

            mu_map.insert(entity, parent_body_mu);
        }

        let mut mean_anomaly_map = HashMap::new();
        for (entity, assoc_entity) in &assoc_entity_map {
            if *assoc_entity == Entity::DANGLING {
                mean_anomaly_map.insert(entity, 0.0);
            } else {
                let assoc_state = self
                    .world
                    .get::<&State>(*assoc_entity)
                    .expect("the associated entity's gotta have state");
                let mu = *mu_map.get(entity).unwrap();

                // hyperbolic orbits don't have a meaningful mean anomaly, use 0
                if assoc_state.ecc(mu) >= 1.0 {
                    mean_anomaly_map.insert(entity, 0.0);
                } else {
                    let mean_anomaly_0 = assoc_state.mean_anomaly(mu); // M at assoc_state.t = vertex 0
                    let state_now = assoc_state.propagate(self.current_et.get(), mu).unwrap();
                    let mean_anomaly = state_now.mean_anomaly(mu);
                    mean_anomaly_map.insert(entity, mean_anomaly - mean_anomaly_0);
                }
            }
        }

        let mut proximity_alphas = HashMap::new();
        for (entity, (_line, _parent)) in self.world.query::<(&LinePathComponent, &Parent)>().iter()
        {
            let assoc_entity = *assoc_entity_map.get(&entity).unwrap();
            let assoc_planet =
                get_ancestor(&self.world, assoc_entity).unwrap_or(self.selection.bodies[0]);

            let camera_dist =
                (pos_map.get(&closest_body).unwrap() - self.camera_3d.world_pos).norm();

            // fade if:
            let fade_orbit = if assoc_entity == assoc_planet {
                // I'm a planet, and camera is close to me
                camera_dist < closest_planet_soi
            } else {
                // I'm a moon/craft, and camera is close to a planet thats not mine
                closest_planet != assoc_planet && closest_dist < closest_planet_soi
            };

            let proximity_alpha = if fade_orbit { 0.0 } else { 1.0 };

            proximity_alphas.insert(entity, proximity_alpha);
        }

        // Set the origins of the line paths wrt the parent world positions
        for (entity, (line, world_pos, parent)) in
            self.world
                .query_mut::<(&mut LinePathComponent, &mut WorldPosition, &Parent)>()
        {
            let parent_pos = pos_map.get(&parent.id).unwrap();

            let selected = match self.selection.selected_entity() {
                Some(selected_entity) => {
                    let assoc_craft = *assoc_entity_map.get(&entity).unwrap();
                    assoc_craft == selected_entity
                }
                None => false,
            };

            line.color = STYLE.accent;

            if selected && !self.selection.is_animating(app.seconds as f64) {
                line.color.w = 1.0;
                line.width = 2.0;
            } else {
                line.color.w = 0.36606;
                line.width = 1.0;
            }

            line.color.w *= proximity_alphas.get(&entity).unwrap();

            let mean_anomaly = mean_anomaly_map.get(&entity).unwrap();
            line.seam = (mean_anomaly / (2.0 * PI)).rem_euclid(1.0) as f32;

            world_pos.pos = *parent_pos;
        }
    }

    fn get_selected_body_radius(&self) -> Option<f64> {
        let entity = self.selection.selected_entity()?;
        let mut q = self.world.query_one::<&Body>(entity).ok()?;
        let body = q.get()?;
        Some(body.body_radius)
    }

    /// Updates the camera position and lookat based on mouse panning and body selection
    fn camera_update(&mut self, app: &App) {
        let rot_matrix = nalgebra_glm::rotate_y(
            &nalgebra_glm::rotate_z(&nalgebra_glm::one(), self.phi),
            self.theta,
        );
        let transition =
            cubic_ease_in_out((app.seconds as f64 - self.selection.transition).min(1.0));
        let offset = (1.0 - transition) * self.selection.prev_selected_pos
            + transition * self.selection.selected_pos;
        self.camera_3d.world_pos =
            (rot_matrix * nalgebra_glm::vec4(self.distance, 0., 0., 0.)).xyz() + offset;
        self.camera_3d.sync(offset);
    }

    fn world_to_screen(&self, relative_pos: DVec3, app: &App) -> Option<Vec2> {
        let window_size = app.window_size;
        let (view, proj) = self.camera_3d.inner.view_proj_matrices();
        let clip = proj
            * view
            * vec4(
                relative_pos.x as f32,
                relative_pos.y as f32,
                relative_pos.z as f32,
                1.0,
            );
        if clip.w <= 0.0 {
            return None;
        } // behind camera
        let ndc = clip.xyz() / clip.w;
        Some(vec2(
            ((ndc.x + 1.0) / 2.0) as f32 * window_size.x as f32,
            ((1.0 - ndc.y) / 2.0) as f32 * window_size.y as f32,
        ))
    }

    fn render_dots(&self, app: &App) {
        app.renderer.set_color(vec4(1.0, 1.0, 1.0, 1.0));

        for (entity, (world_pos, _model)) in self
            .world
            .query::<hecs::Without<(&WorldPosition, &ModelComponent), &SurfaceTile>>()
            .iter()
        {
            let relative_pos = world_pos.pos - self.camera_3d.world_pos;
            if let Some(screen) = self.world_to_screen(relative_pos, app) {
                let rect = Rectangle {
                    pos: screen,
                    size: vec2(2.0, 2.0),
                };
                let radius = self
                    .world
                    .get::<&Body>(entity)
                    .map(|b| b.body_radius)
                    .unwrap_or(0.0);

                if self.apparent_radius_px(radius, relative_pos.norm(), app) < 2.0
                    && !self.is_occluded(entity, relative_pos)
                {
                    app.renderer.fill_rect(rect);
                }
            }
        }
    }

    fn is_occluded(&self, entity: Entity, relative_pos: DVec3) -> bool {
        let dist = relative_pos.norm();
        let dir = relative_pos / dist;

        for (other, (opos, obody)) in self.world.query::<(&WorldPosition, &Body)>().iter() {
            if other == entity {
                continue; // body never occludes itself
            }

            let c = opos.pos - self.camera_3d.world_pos;
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

    /// Radius of a body on screen in px
    fn apparent_radius_px(&self, radius: f64, dist: f64, app: &App) -> f64 {
        let ProjectionKind::Perspective { fov_rad, .. } = self.camera_3d.inner.projection_kind
        else {
            return 0.0;
        };
        (radius / dist) / (fov_rad as f64 / 2.0).tan() * (app.window_size.y as f64 / 2.0)
    }

    fn sync_tile_path(
        slot: &mut Option<(Entity, usize, LinePathComponent)>,
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

    fn selected_tile_key(&self) -> Option<(Entity, usize)> {
        let sel = self.selection.selected_entity()?;

        // a selected building resolves to the tile it stands on
        if let (Ok(tile), Ok(parent)) = (
            self.world.get::<&SurfaceTile>(sel),
            self.world.get::<&Parent>(sel),
        ) {
            return Some((parent.id, tile.index as usize));
        }

        // otherwise a clicked bare tile stays lit while its body is selected
        self.clicked_tile_key.filter(|(body, _)| *body == sel)
    }

    fn sync_selected_tile(&mut self, app: &App) {
        let key = self.selected_tile_key();

        if key.is_none() {
            // clear the clicked tile
            self.clicked_tile_key = None;
        }

        let want = key.and_then(|(b, i)| self.tile_outline_vertices(b, i).map(|v| (b, i, v)));
        Self::sync_tile_path(&mut self.selected_tile, want, 1.0, &app.renderer);
    }

    fn sync_panel(&mut self, app: &App) {
        let now = self.current_et.get();

        let footer_view = FooterView {
            now,
            paused: self.paused,
            speed_label: self.sim_speed.rate_label(),
            can_speed_up: self.sim_speed.can_speed_up(),
            can_slow_down: self.sim_speed.can_slow_down(),
        };
        if self.footer.sync(app, &footer_view) {
            // The side panel's height depends on the footer's
            self.gui_built_for = None;
        }

        let key = self.gui_structure_key();
        if key != self.gui_built_for {
            self.gui_built_for = key;
            let ctx = PanelCtx {
                app,
                world: &self.world,
                parts: &self.parts,
                now,
                controls_enabled: &self.controls_enabled,
            };
            let (gui, bindings) =
                panel::build(&ctx, self.selection.selected_entity(), self.footer.height());
            self.gui = gui;
            self.gui_bindings = bindings;
        }

        for binding in &self.gui_bindings {
            binding.sync(&self.world, now);
        }
    }

    fn tile_outline_vertices(&self, body: Entity, index: usize) -> Option<Vec<f32>> {
        let mut q = self.world.query_one::<(&Body, &TileMap)>(body).ok()?;
        let (b, tile_map) = q.get()?;

        let r = b.body_radius as f32 * 1.002;
        let corners: [f32; 9] = (*tile_map.tris.get(index)? * r).into();

        let mut v = Vec::with_capacity(12);
        v.extend_from_slice(&corners);
        v.extend_from_slice(&corners[0..3]);
        Some(v)
    }
}

/// Cubic easing out function - for animation
fn cubic_ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powf(3.0) / 2.0
    }
}
