//! This module is responsible for defining the gameplay scene.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    f64::consts::PI,
    rc::Rc,
};

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
use nalgebra_glm::{vec2, vec3, vec4, DVec3, I32Vec2, Vec2, Vec3};
use sdl2::keyboard::Scancode;

use crate::{
    astro::{
        epoch::EphemerisTime,
        maneuver::sphere_of_influence,
        state::State,
        units::{EARTH_RADII_PER_AU, KM_PER_EARTH_RADIUS, SUN_MU},
    },
    components::{
        craft::{
            replace_line_path, spawn_orbiting_craft, AssociatedEntity, Command, Payload,
            ScheduledBurn, Stage,
        },
        factory::{projected_completion, Factory},
        inventory::PartInventory,
        parts::{id_hash, PartDef, PartRegistry},
        station::{
            add_resource, commit_station, next_reservoir_limits, resource_store_amount,
            station_r_au, station_resource_amount_flow, station_resource_totals, take_resource,
            Electrolyzer, Resource, ResourceStore, SolarPanel, Station, StationModule,
        },
        tile::{SurfaceTile, TileMap, TileSets},
    },
    container,
    generation::{lexicon::Lexicon, polygon},
    scenes::{
        events::{Event, EventQueue},
        fabricator::{FabricatorAction, FabricatorUi},
        game_over::GameOverUi,
        maneuver::ManeuverModal,
        sim_speed::SimSpeed,
        starbox::Starbox,
        vab::VabUi,
    },
    ui::{
        anchor::{Anchor, AnchorPoint},
        bind::Binding,
        button::{Button, Icon},
        container::{Align, Flow, Justify},
        hrule::HRule,
        label::Label,
        progress_bar::ProgressBar,
        scroll_container::ScrollContainer,
        shape::Shape,
        stat_row::stat_row,
        style::STYLE,
        timeline::{MarkKind, Timeline, TimelineMark},
        toggle::Toggle,
    },
};

use crate::{
    components::{
        body::{spawn_body, Body, Category, Parent, SceneObject},
        craft::{spawn_landed_craft, Craft, Landed},
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

    turn_gui_built_for: Option<(I32Vec2, (u64, u64), Option<EphemerisTime>)>,
    turn_gui: Anchor<TurnMessages>,
    gui: Anchor<CommandMessages>,
    gui_built_for: Option<(Entity, u32, u64, u64)>,
    gui_bindings: Vec<Binding>,
    fabricator_ui: FabricatorUi,
    vab_ui: VabUi,
    maneuver_ui: ManeuverModal,
    game_over_ui: GameOverUi,

    // RCs for GUI
    controls_enabled: Rc<Cell<bool>>,
    turn_progress: Rc<Cell<f32>>,
    calendar_string: Rc<RefCell<String>>,
    marks: Rc<RefCell<Vec<TimelineMark>>>,
    marks_key: (u64, u64),
    transport_icon: Rc<Cell<Icon>>,

    // Events and timeline
    event_queue: EventQueue,
    current_et: Rc<Cell<EphemerisTime>>,
    paused: bool,
    pause_reasons: Vec<TimelineMark>,
    sim_speed: SimSpeed,
    /// Either the next event, or None
    run_until: Option<EphemerisTime>,

    // Vec of unit vectors
    starbox: Starbox,
}

#[derive(Clone)]
enum TurnMessages {
    TogglePlay,
    SpeedUp,
    SlowDown,
}

#[derive(Clone)]
pub enum CommandMessages {
    OpenFabricator {
        fabricator_entity: Entity,
    },
    CancelQueuedFabricator {
        fabricator_entity: Entity,
    },
    CancelActiveFabricator {
        fabricator_entity: Entity,
    },
    ToggleFabricator {
        fabricator_entity: Entity,
    },
    ToggleElectrolyzer {
        electrolyzer_entity: Entity,
    },
    CancelCommand {
        craft: Entity,
    },
    SelectEntity {
        entity: Entity,
    },
    #[allow(unused)]
    FactoryCommand {
        part_id: u64,
        factory_entity: Entity,
    },
    #[allow(unused)]
    OpenVab,
    #[allow(unused)]
    OpenManeuver,
}

#[derive(Default)]
struct Section {
    pub widgets: Vec<Box<dyn Widget<CommandMessages>>>,
    pub bindings: Vec<Binding>,
}

impl Section {
    fn push(&mut self, w: impl Widget<CommandMessages> + 'static) {
        self.widgets.push(Box::new(w));
    }

    fn merge(&mut self, other: Section) {
        self.widgets.extend(other.widgets);
        self.bindings.extend(other.bindings);
    }

    fn into_card(self) -> Section {
        let c = Container::new(self.widgets)
            .fixed_width(vec2(280.0, 0.0))
            .border(STYLE.border, 1.0);
        Section {
            widgets: vec![Box::new(c)],
            bindings: self.bindings,
        }
    }
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
            || self.vab_ui.is_shown()
            || self.maneuver_ui.is_shown()
            || self.game_over_ui.is_shown();

        if self.game_over_ui.update(app) {
            app.running.set(false);
        }

        if let Some(FabricatorAction {
            fabricator,
            part_id,
        }) = self.fabricator_ui.update(app)
        {
            let mut factory = self.world.get::<&mut Factory>(fabricator).unwrap();
            factory.pending_job = Some(part_id)
        }

        if self.vab_ui.update(app) {
            if let Some(selected) = self.selection.selected_entity() {
                let parent = self.world.get::<&Parent>(selected).unwrap().id;

                let payload = self
                    .vab_ui
                    .payload()
                    .expect("the VAB shouldn't allow invalid payloads");

                let stages = self
                    .vab_ui
                    .stages()
                    .into_iter()
                    .collect::<Option<Vec<Stage>>>()
                    .expect("the VAB shouldn't allow invalid stages");

                {
                    let mut inventory = self.world.get::<&mut PartInventory>(parent).unwrap();
                    inventory
                        .take(self.vab_ui.payload.as_ref().unwrap().id_hash())
                        .unwrap();
                    for stage in &self.vab_ui.stages {
                        inventory.take(stage.id_hash()).unwrap()
                    }
                }

                let landed_craft_entity = spawn_landed_craft(
                    payload,
                    stages,
                    SceneObject {
                        bvh_node_id: None,
                        name: String::from("landed craft"),
                    },
                    Parent { id: parent },
                    &mut self.world,
                    &app.renderer,
                    &mut self.bvh,
                );
                self.selection.crafts.push(landed_craft_entity);
            }
        }

        if let Some(result) = self
            .maneuver_ui
            .update(self.current_et.get(), &self.world, app)
        {
            if let Some(selected) = self.selection.selected_entity() {
                let command = result.into_command();
                self.world.get::<&mut Craft>(selected).unwrap().command = Some(command);
            }
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
                    }
                    CommandMessages::CancelActiveFabricator { fabricator_entity } => {
                        let mut factory =
                            self.world.get::<&mut Factory>(fabricator_entity).unwrap();
                        factory.current_job = None;
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
                    CommandMessages::SelectEntity { entity } => {
                        self.selection.set_selected(entity, app.seconds as f64);
                    }
                    CommandMessages::FactoryCommand {
                        part_id,
                        factory_entity,
                    } => {
                        self.world
                            .get::<&mut Factory>(factory_entity)
                            .unwrap()
                            .start_job(part_id, self.current_et.get(), 1.0)
                            .expect("you wouldnt give a fake part would you");
                    }
                    CommandMessages::OpenVab => {
                        if let Some(selected) = self.selection.selected_entity() {
                            let parent = self.world.get::<&Parent>(selected).unwrap().id;
                            let inventory = self.world.get::<&PartInventory>(parent).unwrap();
                            self.vab_ui.show(&inventory, &self.parts, app);
                        }
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
                }
            }

            for msg in recv_msgs(app, &mut self.turn_gui) {
                match msg {
                    TurnMessages::TogglePlay => {
                        let now = self.current_et.get();
                        self.commit_pending_builds(now);
                        self.recompute_run_until();
                        self.paused = !self.paused;
                        if !self.paused {
                            self.pause_reasons.clear();
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
                self.pause_reasons = self
                    .marks
                    .borrow()
                    .iter()
                    .filter(|m| m.t <= t)
                    .cloned()
                    .collect();

                for event in self.event_queue.pop_due(t) {
                    self.handle_event(event, app);
                }
                self.complete_due_jobs(t, app);
                self.recompute_run_until();
            }

            if !self.game_over_ui.is_shown() {
                if let Some((station, cause)) = self.crew_death() {
                    let now = self.current_et.get();
                    commit_station(&self.world, station, now);
                    self.world.get::<&mut Station>(station).unwrap().num_crew = 0;
                    self.paused = true;
                    self.run_until = None;

                    let name = self
                        .world
                        .get::<&SceneObject>(station)
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                    self.game_over_ui.show(&name, cause, now, app);
                }
            }
        }

        // Update GUI stuff
        let cal = self.current_et.get().short_date();
        if *self.calendar_string.borrow() != cal {
            *self.calendar_string.borrow_mut() = cal;
        }
        self.controls_enabled.set(self.paused);
        self.transport_icon
            .set(if self.paused { Icon::Play } else { Icon::Pause });

        self.orbit_system();
        self.landed_system();
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
        let key = (self.event_queue.version(), self.job_state_bits());
        if key != self.marks_key {
            self.marks_key = key;
            *self.marks.borrow_mut() = self.build_marks();
        }
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
                let scene_obj = self.world.get::<&SceneObject>(hovered).unwrap();

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
                            .draw_text(screen_pos + vec2(8.0, 8.0), &scene_obj.name);
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
        self.turn_gui.render(app);
        self.fabricator_ui.render(app);
        self.vab_ui.render(app);
        self.maneuver_ui.render(app);
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
            SceneObject {
                bvh_node_id: None,
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
                SceneObject {
                    bvh_node_id: None,
                    name,
                },
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
                    SceneObject {
                        bvh_node_id: None,
                        name,
                    },
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

        let station_payload = parts
            .all()
            .find(|p| p.id == "station_core")
            .unwrap()
            .instantiate_payload();

        let station = spawn_orbiting_craft(
            station_payload,
            vec![],
            SceneObject {
                bvh_node_id: None,
                name: String::from("Station"),
            },
            Parent { id: station_parent },
            State::from_kepler(
                parent_body_radius * 16.0,
                0.2,
                0.0,
                1.5,
                0.15,
                0.15,
                EphemerisTime::new(0),
                parent_mu,
            ),
            &mut world,
            &app.renderer,
            &mut bvh,
        );

        let mut starting_inventory = PartInventory {
            parts: HashMap::new(),
        };

        starting_inventory.add(id_hash("ilmenite"), 8);

        world
            .insert(
                station,
                (
                    Station {
                        num_crew: 2,
                        modules_gen: 0,
                    },
                    starting_inventory,
                ),
            )
            .unwrap();
        world.spawn((
            StationModule { slot: 0 },
            ResourceStore {
                resource: Resource::Energy,
                amount: 4.32e8,
                capacity: 1.8e9,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 1 },
            SolarPanel { rated_w: 100_000.0 },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 2 },
            ResourceStore {
                resource: Resource::Water,
                amount: 3800.0,
                capacity: 3800.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 3 },
            ResourceStore {
                resource: Resource::Oxygen,
                amount: 600.0,
                capacity: 600.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 4 },
            ResourceStore {
                resource: Resource::Hydrogen,
                amount: 100.0,
                capacity: 100.0,
                amount_et: EphemerisTime::epoch(),
            },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 5 },
            Factory {
                current_job: None,
                pending_job: None,
                power_watts: 5000.0,
                enabled: false,
            },
            Parent { id: station },
        ));
        world.spawn((
            StationModule { slot: 6 },
            Electrolyzer {
                enabled: true,
                power_watts: 5_000.0,
                joules_per_kg_water: 2.52e7,
            },
            Parent { id: station },
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

            turn_gui_built_for: None,
            gui: Anchor::new(Box::new(container![]), AnchorPoint::TopRight),
            gui_built_for: None,
            gui_bindings: vec![],
            turn_gui: Anchor::new(Box::new(container![]), AnchorPoint::BottomLeft),
            fabricator_ui: FabricatorUi::new(),
            vab_ui: VabUi::new(),
            maneuver_ui: ManeuverModal::new(app),
            game_over_ui: GameOverUi::new(),

            controls_enabled: Rc::new(Cell::new(false)),
            turn_progress: Rc::new(Cell::new(0.0)),
            calendar_string: Rc::new(RefCell::new(String::new())),
            marks: Rc::new(RefCell::new(vec![])),
            marks_key: (event_queue.version(), 0),
            transport_icon: Rc::new(Cell::new(Icon::Play)),

            current_et: Rc::new(Cell::new(EphemerisTime::epoch())),
            event_queue,
            paused: true,
            pause_reasons: vec![],
            sim_speed: SimSpeed::new(),
            run_until: None,

            starbox: Starbox::new(9000, vec3(1.0, 2.0, 4.0), 0.4),
        };

        *retval.calendar_string.borrow_mut() = retval.current_et.get().short_date();

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

    fn rebuild_gui(&mut self, app: &App) -> (Anchor<CommandMessages>, Vec<Binding>) {
        let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![];
        let mut bindings = vec![];
        let selected = self.selection.selected_entity();

        if let Some(selected) = selected {
            let section = self.build_selection_widgets(selected, app);
            widgets = section.widgets;
            bindings = section.bindings;
        }

        const MARGIN: f32 = 16.0;

        let footer_h = self.turn_gui.size().y;
        let panel_h = (app.window_size.y as f32 - MARGIN * 3.0 - footer_h).max(0.0);

        let mut anchor = Anchor::new(
            Box::new(ScrollContainer::new(
                Vec2::new(300.0, panel_h),
                Box::new(
                    Container::new(widgets)
                        .cross_align(Align::Start)
                        .background_color(STYLE.surface)
                        .border(STYLE.border, 1.0)
                        .padding(vec2(8.0, 8.0))
                        .min_size(Vec2::new(300.0, panel_h)),
                ),
            )),
            AnchorPoint::TopRight,
        )
        .margin(vec2(MARGIN, MARGIN));
        anchor.reposition(app);
        (anchor, bindings)
    }

    fn rebuild_turn_gui(&mut self, app: &App) -> Anchor<TurnMessages> {
        let mut turn_widgets: Vec<Box<dyn Widget<TurnMessages>>> = vec![];
        turn_widgets.extend(self.build_footer_widgets(app));
        const MARGIN: f32 = 16.0;

        let mut anchor = Anchor::new(
            Box::new(
                Container::new(turn_widgets)
                    .padding(vec2(0.0, 0.0))
                    .gap(MARGIN)
                    .cross_align(Align::End)
                    .flow(Flow::Horizontal),
            ),
            AnchorPoint::BottomRight,
        )
        .margin(vec2(MARGIN, MARGIN));
        anchor.reposition(app);
        anchor
    }

    fn build_footer_widgets(&self, app: &App) -> Vec<Box<dyn Widget<TurnMessages>>> {
        let font = app.renderer.get_font_id_from_name("font").unwrap();

        const WIDTH: f32 = 300.0;
        const LIST_H: f32 = 85.0;

        let now = self.current_et.get();
        let marks = self.marks.borrow();
        let mut upcoming: Vec<&TimelineMark> = marks.iter().filter(|m| m.t > now).collect();
        upcoming.sort_by_key(|m| m.t);

        let rows = self
            .pause_reasons
            .iter()
            .map(|m| self.event_row(m, true, app))
            .chain(upcoming.iter().map(|m| self.event_row(m, false, app)))
            .collect();

        let turn_controls = Container::new(vec![
            Box::new(ScrollContainer::new(
                Vec2::new(WIDTH - 16.0, LIST_H),
                Box::new(Container::new(rows).padding(Vec2::zeros()).gap(2.0)),
            )),
            Box::new(HRule::new(STYLE.border, 1.0, WIDTH - 16.0)),
            Box::new(
                Container::new(vec![
                    Box::new(
                        Button::icon_bound(vec2(30.0, 30.0), self.transport_icon.clone())
                            .use_style_accented(&STYLE)
                            .on_click(TurnMessages::TogglePlay),
                    ),
                    Box::new(
                        Button::icon(vec2(30.0, 30.0), Icon::SlowForward)
                            .use_style_accented(&STYLE)
                            .bound_active(self.sim_speed.can_slow_down.clone())
                            .on_click(TurnMessages::SlowDown),
                    ),
                    Box::new(
                        Button::icon(vec2(30.0, 30.0), Icon::FastForward)
                            .use_style_accented(&STYLE)
                            .bound_active(self.sim_speed.can_speed_up.clone())
                            .on_click(TurnMessages::SpeedUp),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Label::bound(self.sim_speed.sim_speed_str.clone()).font(font, app),
                        )])
                        .background_color(STYLE.surface)
                        .border(STYLE.border, 1.0)
                        .min_size(vec2(0.0, 30.0)),
                    ),
                    Box::new(
                        Container::new(vec![Box::new(
                            Label::bound(self.calendar_string.clone()).font(font, app),
                        )])
                        .background_color(STYLE.surface)
                        .border(STYLE.border, 1.0)
                        .min_size(vec2(0.0, 30.0)),
                    ),
                ])
                .flow(Flow::Horizontal)
                .cross_align(Align::Center)
                .padding(vec2(0.0, 0.0))
                .gap(0.0),
            ),
        ])
        .background_color(STYLE.surface)
        .border(STYLE.border, 1.0)
        .flow(Flow::Vertical)
        .cross_align(Align::Center)
        .fixed_size(Vec2::new(300.0, 150.0));

        let remaining = app.window_size.x as f32 - turn_controls.size().x - 16.0 - 32.0;

        vec![
            Box::new(
                Timeline::new(self.current_et.clone(), remaining, self.marks.clone())
                    .use_style(&STYLE),
            ),
            Box::new(turn_controls),
        ]
    }

    fn event_row(
        &self,
        mark: &TimelineMark,
        accented: bool,
        app: &App,
    ) -> Box<dyn Widget<TurnMessages>> {
        let font = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let (mesh, rot) = mark.kind.shape(app);

        const WIDTH: f32 = 300.0;

        let (subject_color, detail_color) = match (accented, mark.kind) {
            (true, MarkKind::Critical) => (STYLE.negative, STYLE.text_secondary),
            (true, _) => (STYLE.positive, STYLE.text_secondary),
            (false, _) => (STYLE.text, STYLE.text_secondary),
        };

        let left = container!(
            Shape::new(vec2(28.0, 28.0), mesh, rot, mark.kind.color(), 10.0),
            container!(
                container!(
                    Label::new(mark.subject.clone())
                        .font(font, app)
                        .color(subject_color),
                    Label::new(mark.t.short_datetime()).font(font, app),
                )
                .flow(Flow::Horizontal)
                .justify(Justify::SpaceBetween)
                .fixed_size(vec2(WIDTH - 52.0, 16.0))
                .padding(Vec2::zeros())
                .gap(0.0),
                Label::new(mark.detail.clone())
                    .font(font, app)
                    .color(detail_color)
            )
            .padding(Vec2::zeros())
            .gap(0.0)
        )
        .flow(Flow::Horizontal)
        .cross_align(Align::Start)
        .padding(Vec2::zeros())
        .gap(6.0);

        Box::new(
            container!(left)
                .flow(Flow::Horizontal)
                .justify(Justify::SpaceBetween)
                .cross_align(Align::Start)
                .padding(Vec2::zeros())
                .fixed_width(vec2(WIDTH - 16.0, 0.0)),
        )
    }

    fn build_selection_widgets(&self, selected: Entity, app: &App) -> Section {
        let mut out = Section::default();
        const WIDTH: f32 = 280.0;
        let font_big = app.renderer.get_font_id_from_name("font-big").unwrap();

        out.push(
            Container::new(self.build_crumbs(selected, app))
                .flow(Flow::Horizontal)
                .cross_align(Align::Center)
                .padding(vec2(0.0, 0.0)),
        );

        let name = self
            .world
            .get::<&SceneObject>(selected)
            .map(|n| n.name.clone())
            .unwrap_or_else(|_| "???".into());
        out.push(Label::new(name).font(font_big, app));
        out.push(HRule::new(STYLE.border, 1.0, WIDTH));

        if self.world.get::<&Station>(selected).is_ok() {
            out.merge(self.station_selection(selected, app));
        } else if self.world.get::<&Craft>(selected).is_ok() {
            out.merge(self.craft_selection(selected, app));
        } else if self.world.get::<&Body>(selected).is_ok() {
            out.merge(self.body_selection(selected, app));
        }

        out
    }

    fn craft_selection(&self, selected: Entity, app: &App) -> Section {
        let mut out = Section::default();

        const WIDTH: f32 = 280.0;
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font_small_italic = app
            .renderer
            .get_font_id_from_name("font-small-italic")
            .unwrap();

        let craft = self.world.get::<&Craft>(selected).unwrap();

        let craft_dv = craft.total_remaining_dv();

        let is_idle = craft.command.is_none();

        let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> =
            vec![Box::new(Label::new("MISSION").font(font_small_bold, app))];

        if let Some(command) = &craft.command {
            let (verb, target) = command.title_parts();
            let name = self
                .world
                .get::<&SceneObject>(target)
                .ok()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "???".into());

            let title = format!("{verb} {name}").to_uppercase();

            let header = if craft.command_scheduled {
                title
            } else {
                format!("{} - QUEUED", title)
            };
            widgets.push(Box::new(
                Label::new(header)
                    .font(font_small_bold, app)
                    .color(STYLE.accent),
            ));

            for burn in command.burn_schedule() {
                let card = self.burn_card(&burn, app);
                widgets.extend(card.widgets);
                out.bindings.extend(card.bindings);
            }

            if !craft.command_scheduled {
                widgets.push(Box::new(
                    Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Cancel Mission")
                        .use_style(&STYLE)
                        .bound_active(self.controls_enabled.clone())
                        .on_click(CommandMessages::CancelCommand { craft: selected }),
                ));
            }
        } else {
            widgets.push(Box::new(
                Label::new("NO MISSION ASSIGNED")
                    .font(font_small_italic, app)
                    .color(STYLE.text_disabled),
            ));
        }

        if is_idle {
            widgets.push(Box::new(
                Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Plan Mission...")
                    .use_style_accented(&STYLE)
                    .on_click(CommandMessages::OpenManeuver),
            ))
        };
        widgets.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));

        // Stages from bottom to top
        widgets.push(Box::new(Label::new("STAGES").font(font_small_bold, app)));

        widgets.push(Box::new(
            Label::new(format!("Total dv: {:.0} m/s", craft_dv))
                .font(font, app)
                .color(STYLE.text),
        ));

        for stage in craft.stages_stack.iter() {
            let fuel_pct = stage.fuel_mass / stage.max_fuel_mass;
            widgets.push(Box::new(
                Container::new(vec![
                    Box::new(Label::new(stage.name.clone()).font(font_small_bold, app)),
                    Box::new(
                        Label::new(format!(
                            "{:.0}/{:.0} kg",
                            stage.fuel_mass, stage.max_fuel_mass
                        ))
                        .font(font, app)
                        .color(if fuel_pct > 0.25 {
                            STYLE.text
                        } else {
                            STYLE.warning
                        }),
                    ),
                    Box::new(
                        ProgressBar::new(vec2(WIDTH - 24.0, 8.0))
                            .use_style(&STYLE)
                            .progress(fuel_pct as f32),
                    ),
                ])
                .flow(Flow::Vertical)
                .border(STYLE.border, 1.0)
                .fixed_width(vec2(WIDTH, 10.0)),
            ));
        }

        out.widgets = widgets;
        out
    }

    fn burn_card(&self, burn: &ScheduledBurn, app: &App) -> Section {
        const WIDTH: f32 = 280.0;
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        let now = self.current_et.get();
        let done = now >= burn.t();
        let (title_color, date_color) = if done {
            (STYLE.positive, STYLE.positive)
        } else {
            (STYLE.text, STYLE.text_secondary)
        };

        let countdown = Rc::new(RefCell::new(String::new()));

        let mut out = Section::default();
        out.push(
            Container::new(vec![
                Box::new(
                    Container::new(vec![
                        Box::new(
                            Label::new(burn.desc)
                                .font(font_small_bold, app)
                                .color(title_color),
                        ),
                        Box::new(
                            Label::new(format!("{:.0} m/s", burn.dv))
                                .font(font_small_bold, app)
                                .color(title_color),
                        ),
                    ])
                    .flow(Flow::Horizontal)
                    .justify(Justify::SpaceBetween)
                    .padding(Vec2::zeros())
                    .fixed_width(vec2(WIDTH - 16.0, 0.0)),
                ),
                Box::new(
                    Label::new(burn.t().as_calendar())
                        .font(font, app)
                        .color(date_color),
                ),
                Box::new(
                    Label::bound(countdown.clone())
                        .font(font, app)
                        .color(date_color),
                ),
            ])
            .border(STYLE.border, 1.0)
            .fixed_width(vec2(WIDTH, 0.0))
            .padding(vec2(8.0, 8.0)),
        );

        let current_et = self.current_et.clone();
        out.bindings.push(Binding::new({
            let burn_t = burn.t();
            move |_world| {
                let now = current_et.get();
                let s = if now >= burn_t {
                    "DONE".into()
                } else {
                    format!("T- {}", (burn_t - now).short_duration())
                };
                if *countdown.borrow() != s {
                    *countdown.borrow_mut() = s;
                }
            }
        }));
        out
    }

    fn body_selection(&self, selected: Entity, app: &App) -> Section {
        let mut out = Section::default();
        const WIDTH: f32 = 280.0;

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        let body = self.world.get::<&Body>(selected).unwrap();
        let inventory = self.world.get::<&PartInventory>(selected).unwrap();

        let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![];

        if let Ok(parent) = self.world.get::<&Parent>(selected) {
            let state = self.world.get::<&State>(selected).unwrap();
            let parent_body = self.world.get::<&Body>(parent.id).unwrap();

            let orbits_star = self.world.get::<&Parent>(parent.id).is_err();
            let dist = |er: f64| {
                if orbits_star {
                    format!("{:.2} AU", er / EARTH_RADII_PER_AU)
                } else {
                    format!("{:.0} km", er * KM_PER_EARTH_RADIUS)
                }
            };

            let (apo, peri) = state.apsides(parent_body.mu);
            let ecc = state.ecc(parent_body.mu);
            let inc = state.inclination();

            let orbit_rows = [
                ("APOAPSIS", apo.map_or("-".into(), dist)),
                ("PERIAPSIS", dist(peri)),
                ("ECCENTRICITY", format!("{:.3}", ecc)),
                ("INCLINATION", format!("{:.1} deg", inc.to_degrees())),
            ];

            widgets.extend(orbit_rows.iter().map(|(k, v)| {
                Box::new(stat_row(k, v.to_string(), STYLE.text, WIDTH, app))
                    as Box<dyn Widget<CommandMessages>>
            }));
            widgets.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));
        }

        let body_rows = [
            // TODO: Support earth symbol and exponents in apricot's font cache
            ("RADIUS", format!("{:.1} ER", body.body_radius)),
            ("MASS", format!("{:.3} EM", body.mass())),
            ("DENSITY", format!("{:.1} g/cm^3", body.density)),
            ("DAY", format!("{:.1} hrs", body.rotation_period_hours)),
            // TODO: Replace the following with sensor estimates
            ("PRESSURE", String::from("-")),
            ("TEMPERATURE", String::from("-")),
            ("CORE MASS", String::from("-")),
            ("MAGNETIC", String::from("-")),
        ];

        // let state = self.world.get::<&State>(selected).unwrap();
        // Know: name, radius, mass, density, orbital radius, rotation in hours
        // Have to find: atmos press, temp, core mass fraction, magnetic field
        widgets.extend(body_rows.iter().map(|(k, v)| {
            Box::new(stat_row(k, v.to_string(), STYLE.text, WIDTH, app))
                as Box<dyn Widget<CommandMessages>>
        }));

        // Extend with inventory info
        widgets.extend(inventory.parts.iter().filter_map(|(part_id, quantity)| {
            if *quantity > 0 {
                Some(
                    Box::new(Label::new(format!("{}: {}", part_id, quantity)).font(font, app))
                        as Box<dyn Widget<CommandMessages>>,
                )
            } else {
                None
            }
        }));
        out.widgets = widgets;

        let children: Vec<Entity> = self
            .world
            .query::<(&Parent, &Body)>()
            .iter()
            .filter(|(_, (p, _))| p.id == selected)
            .map(|(e, _)| e)
            .collect();

        if !children.is_empty() {
            let has_parent = self.world.get::<&Parent>(selected).is_ok();
            out.push(HRule::new(STYLE.border, 1.0, WIDTH));
            out.push(
                Label::new(if has_parent { "MOONS" } else { "PLANETS" }).font(font_small_bold, app),
            );
            out.widgets.extend(children.iter().filter_map(|e| {
                let child_scene_obj = self.world.get::<&SceneObject>(*e).ok()?;
                Some(Box::new(
                    Button::fit(&child_scene_obj.name, font, app, vec2(0.0, 0.0))
                        .use_style_link(&STYLE)
                        .on_click(CommandMessages::SelectEntity { entity: *e }),
                ) as Box<dyn Widget<CommandMessages>>)
            }));
        }

        let craft: Vec<Entity> = self
            .world
            .query::<(&Parent, &Craft)>()
            .iter()
            .filter(|(_, (p, _))| p.id == selected)
            .map(|(e, _)| e)
            .collect();

        if !craft.is_empty() {
            out.push(HRule::new(STYLE.border, 1.0, WIDTH));
            out.push(Label::new("CRAFT").font(font_small_bold, app));
            out.widgets.extend(craft.iter().filter_map(|e| {
                let child_scene_obj = self.world.get::<&SceneObject>(*e).ok()?;
                Some(Box::new(
                    Button::fit(&child_scene_obj.name, font, app, vec2(0.0, 0.0))
                        .use_style_link(&STYLE)
                        .on_click(CommandMessages::SelectEntity { entity: *e }),
                ) as Box<dyn Widget<CommandMessages>>)
            }));
        }

        out
    }

    #[allow(unused)]
    fn build_factory_info(
        &self,
        selected: Entity,
        app: &App,
    ) -> Vec<Box<dyn Widget<CommandMessages>>> {
        const WIDTH: f32 = 280.0;

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font_big = app.renderer.get_font_id_from_name("font-big").unwrap();
        let factory = self.world.get::<&Factory>(selected).unwrap();

        let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![
            Box::new(Label::new("FACTORY").font(font_big, app)),
            Box::new(HRule::new(STYLE.border, 1.0, WIDTH)),
        ];

        if let Some(job) = &factory.current_job {
            let part = self.parts.get(job.part_id).expect("should be a valid part");

            widgets.extend(vec![
                Box::new(Label::new("STATUS").font(font_small_bold, app)),
                Box::new(Label::new(format!("Building: {}", part.name)).font(font, app))
                    as Box<dyn Widget<CommandMessages>>,
                Box::new(
                    ProgressBar::new(vec2(WIDTH, 12.0))
                        .use_style(&STYLE)
                        .bind(self.turn_progress.clone()),
                ) as Box<dyn Widget<CommandMessages>>,
                Box::new(
                    Label::new(format!(
                        "Completion: {}",
                        job.completion_et(&factory, self.current_et.get())
                            .map_or(String::from("None"), |et| et.as_calendar())
                    ))
                    .font(font, app),
                ) as Box<dyn Widget<CommandMessages>>,
            ])
        } else {
            widgets.extend(vec![
                Box::new(Label::new("STATUS").font(font_small_bold, app)),
                Box::new(Label::new("No orders").font(font, app))
                    as Box<dyn Widget<CommandMessages>>,
                Box::new(HRule::new(STYLE.border, 1.0, WIDTH)) as Box<dyn Widget<CommandMessages>>,
                Box::new(Label::new("BUILD").font(font_small_bold, app))
                    as Box<dyn Widget<CommandMessages>>,
            ]);
            let build_orders: Vec<Box<dyn Widget<CommandMessages>>> = self
                .parts
                .all()
                // TODO: Filter against what this factory can build, SWAPC wise
                .map(|part| {
                    let can_afford = true; // TODO: Check against inventory's parts
                    Box::new(
                        Container::new(vec![
                            Box::new(
                                Container::new(vec![Box::new(
                                    Label::new(part.name.clone())
                                        .font(font_small_bold, app)
                                        .color(if can_afford {
                                            STYLE.text
                                        } else {
                                            STYLE.text_disabled
                                        }),
                                )])
                                .flow(Flow::Vertical)
                                .padding(vec2(0.0, 4.0))
                                .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0)),
                            ),
                            Box::new(
                                Container::new(vec![Box::new(
                                    Button::<CommandMessages>::text(vec2(45.0, 25.0), "BUILD")
                                        .use_style(&STYLE)
                                        .on_click(CommandMessages::FactoryCommand {
                                            part_id: part.id_hash(),
                                            factory_entity: self
                                                .selection
                                                .selected_entity()
                                                .unwrap(),
                                        })
                                        .bound_active(self.controls_enabled.clone())
                                        .active(can_afford),
                                )])
                                .padding(vec2(0.0, 0.0))
                                .fixed_width(vec2(WIDTH * 0.2, 10.0))
                                .cross_align(Align::End)
                                .flow(Flow::Vertical),
                            ),
                        ])
                        .border(STYLE.border, 1.0)
                        .cross_align(Align::Center)
                        .flow(Flow::Horizontal),
                    ) as Box<dyn Widget<CommandMessages>>
                })
                .collect();
            widgets.push(Box::new(
                Container::new(build_orders).padding(vec2(0.0, 0.0)),
            ));
        }

        widgets
    }

    #[allow(unused)]
    fn build_vab_info(&self, selected: Entity, app: &App) -> Vec<Box<dyn Widget<CommandMessages>>> {
        const WIDTH: f32 = 280.0;

        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font_big = app.renderer.get_font_id_from_name("font-big").unwrap();

        let parent = self.world.get::<&Parent>(selected).unwrap().id;
        let inventory = self.world.get::<&PartInventory>(parent).unwrap();

        let mut widgets: Vec<Box<dyn Widget<CommandMessages>>> = vec![
            Box::new(Label::new("VAB").font(font_big, app)),
            Box::new(HRule::new(STYLE.border, 1.0, WIDTH)),
            Box::new(Label::new("INVENTORY").font(font_small_bold, app)),
        ];

        // Show available parts
        let inventory_rows: Vec<Box<dyn Widget<CommandMessages>>> = self
            .parts
            .all()
            .filter_map(|part| {
                let count = inventory.parts.get(&part.id_hash()).copied().unwrap_or(0);
                if count == 0 {
                    return None;
                }
                Some(Box::new(
                    Container::new(vec![
                        Box::new(
                            Container::new(vec![Box::new(
                                Label::new(part.name.clone()).font(font_small_bold, app),
                            )])
                            .flow(Flow::Vertical)
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.8 - 24.0, 10.0)),
                        ),
                        Box::new(
                            Container::new(vec![Box::new(
                                Label::new(format!("x{count}")).font(font, app),
                            )])
                            .padding(vec2(0.0, 0.0))
                            .fixed_width(vec2(WIDTH * 0.2, 10.0))
                            .cross_align(Align::End)
                            .flow(Flow::Vertical),
                        ),
                    ])
                    .border(STYLE.border, 1.0)
                    .cross_align(Align::Center)
                    .flow(Flow::Horizontal),
                ) as Box<dyn Widget<CommandMessages>>)
            })
            .collect();

        if inventory_rows.is_empty() {
            widgets.push(Box::new(
                Label::new("No parts available")
                    .font(font, app)
                    .color(STYLE.text_disabled),
            ));
        } else {
            widgets.push(Box::new(
                Container::new(inventory_rows).padding(vec2(0.0, 0.0)),
            ));
        }

        widgets.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));
        widgets.push(Box::new(Label::new("ASSEMBLE").font(font_small_bold, app)));
        widgets.push(Box::new(
            Button::<CommandMessages>::text(vec2(WIDTH, 30.0), "Stack New Vehicle...")
                .use_style_accented(&STYLE)
                .bound_active(self.controls_enabled.clone())
                .on_click(CommandMessages::OpenVab)
                .active(!inventory.parts.is_empty()),
        ));

        widgets
    }

    fn build_crumbs(&self, selected: Entity, app: &App) -> Vec<Box<dyn Widget<CommandMessages>>> {
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let ancestors = self.ancestor_chain(selected); // outermost first
        let mut crumbs: Vec<Box<dyn Widget<CommandMessages>>> = vec![];
        for (i, e) in ancestors.iter().enumerate() {
            if i > 0 {
                crumbs.push(Box::new(Label::new(">").font(font, app)));
            }
            let name = self.world.get::<&SceneObject>(*e).unwrap().name.clone();
            crumbs.push(Box::new(
                Button::fit(name, font, app, vec2(0.0, 0.0))
                    .use_style_link(&STYLE)
                    .on_click(CommandMessages::SelectEntity { entity: *e }),
            ));
        }
        crumbs.reverse();
        crumbs
    }

    fn ancestor_chain(&self, mut selected: Entity) -> Vec<Entity> {
        let mut ancestors = vec![];
        // Who is that man in my family who said I'll fail?
        while let Ok(parent) = self.world.get::<&Parent>(selected) {
            // finish eating, and come back again!
            ancestors.push(parent.id);
            selected = parent.id;
        }
        // maybe your food is talking to you!
        ancestors
    }

    fn station_selection(&self, station: Entity, app: &App) -> Section {
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        let mut out = Section::default();

        // List of all the modules
        out.push(Label::new("MODULES").font(font_small_bold, app));
        out.merge(self.module_list(station, app));

        out
    }

    fn module_list(&self, station: Entity, app: &App) -> Section {
        let mut modules: Vec<(u32, Entity)> = self
            .world
            .query::<(&StationModule, &Parent)>()
            .iter()
            .filter(|(_, (_, p))| p.id == station)
            .map(|(e, (m, _))| (m.slot, e))
            .collect();
        modules.sort_by_key(|(slot, _)| *slot);

        let mut out = Section::default();

        for (_, module) in modules {
            out.merge(self.module_section(module, app).into_card());
        }
        out
    }

    fn module_section(&self, module: Entity, app: &App) -> Section {
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();

        if self.world.get::<&SolarPanel>(module).is_ok() {
            return self.solar_panel_section(module, app);
        } else if self.world.get::<&ResourceStore>(module).is_ok() {
            return self.resource_store_section(module, app);
        } else if self.world.get::<&Factory>(module).is_ok() {
            return self.fabricator_section(module, app);
        } else if self.world.get::<&Electrolyzer>(module).is_ok() {
            return self.electrolyzer_section(module, app);
        }

        let mut out = Section::default();
        out.push(Label::new("Unknown module!!!").font(font_small_bold, app));
        out
    }

    fn solar_panel_section(&self, module: Entity, app: &App) -> Section {
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let text = Rc::new(RefCell::new(String::new()));

        let mut out = Section::default();

        out.push(Label::new("SOLAR PANEL ARRAY").font(font_small_bold, app));
        out.push(Label::bound(text.clone()).font(font, app));

        out.bindings.push(Binding::new({
            let text = text.clone();
            let last = Cell::new(f32::NAN);
            move |world: &World| {
                let station = world.get::<&Parent>(module).unwrap().id;
                let r_au = station_r_au(world, station);
                let Ok(panel) = world.get::<&SolarPanel>(module) else {
                    return;
                };
                let kw = panel.output_w(r_au) * Resource::Energy.presentation_scalars().1;
                if kw != last.get() {
                    last.set(kw);
                    *text.borrow_mut() = format!("{kw:+.2} kW")
                }
            }
        }));

        out
    }

    fn resource_store_section(&self, module: Entity, app: &App) -> Section {
        const WIDTH: f32 = 280.0;
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font = app.renderer.get_font_id_from_name("font").unwrap();

        let mut out = Section::default();

        // Tank info
        let tank = self.world.get::<&ResourceStore>(module).unwrap();

        out.push(
            Label::new(tank.resource.long_name().to_uppercase().to_string())
                .font(font_small_bold, app),
        );

        let mass = Rc::new(RefCell::new(String::new()));
        let mass_percentage = Rc::new(Cell::new(0.0));
        let time_to_zero = Rc::new(RefCell::new(String::new()));
        let et = self.current_et.clone();

        out.push(Label::bound(mass.clone()).font(font, app).color(STYLE.text));
        out.push(
            ProgressBar::new(vec2(WIDTH - 8.0 * 2.0, 12.0))
                .use_style(&STYLE)
                .bind(mass_percentage.clone()),
        );
        out.push(
            Label::bound(time_to_zero.clone())
                .font(font, app)
                .color(STYLE.text),
        );

        out.bindings.push(Binding::new({
            let mass = mass.clone();
            let last_m = Cell::new(f32::NAN);
            let last_mdot = Cell::new(f32::NAN);
            move |world: &World| {
                let station = world.get::<&Parent>(module).unwrap().id;
                let Ok(t) = world.get::<&ResourceStore>(module) else {
                    return;
                };

                let amount = resource_store_amount(world, module, et.get());
                let rate = station_resource_amount_flow(world, station, t.resource, true);

                let until = |secs: f32| EphemerisTime::from_secs(secs as f64).short_duration();

                let (unit, dunit) = t.resource.presentation_units();
                let (scale, dscale) = t.resource.presentation_scalars();

                let m = amount * scale;
                let mdot = rate * dscale;
                let capacity = t.capacity * scale;

                if m != last_m.get() || mdot != last_mdot.get() {
                    last_m.set(m);
                    last_mdot.set(mdot);
                    *mass.borrow_mut() = format!(
                        "{}: {:.0}/{:.0} {} ({:+.2} {})",
                        t.resource.short_name(),
                        m,
                        capacity,
                        unit,
                        mdot,
                        dunit
                    );
                    mass_percentage.set(m / capacity);
                    *time_to_zero.borrow_mut() = if m == 0.0 {
                        "Empty".into()
                    } else if rate < 0.0 {
                        format!("Empty in {}", until(amount / -rate))
                    } else if rate > 0.0 && amount < t.capacity {
                        format!("Full in {}", until((t.capacity - amount) / rate))
                    } else if rate > 0.0 {
                        "Full - venting".into()
                    } else if amount >= t.capacity {
                        "Full".into()
                    } else {
                        "Stable".into()
                    };
                }
            }
        }));

        out
    }

    fn fabricator_section(&self, module: Entity, app: &App) -> Section {
        const WIDTH: f32 = 280.0;
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font = app.renderer.get_font_id_from_name("font").unwrap();

        let mut out = Section::default();

        let factory = self.world.get::<&Factory>(module).unwrap();
        let enabled = Rc::new(Cell::new(true));
        let progress = Rc::new(Cell::new(0.0));
        let countdown = Rc::new(RefCell::new(String::new()));
        let power_draw = Rc::new(RefCell::new(String::new()));
        let now = self.current_et.get();

        out.push(Label::new(String::from("FABRICATOR")).font(font_small_bold, app));
        out.push(Label::bound(power_draw.clone()).font(font, app));

        if let Some(job) = &factory.current_job {
            let part_name = &self.parts.get(job.part_id).unwrap().name;

            let ready_text = match job.completion_et(&factory, now) {
                Some(et) => format!("Ready: {}", et.as_calendar()),
                None => String::from("Ready:"),
            };

            out.push(Label::new(format!("Building {}", part_name)).font(font, app));
            out.push(
                Toggle::new("Enabled:")
                    .bind(enabled.clone())
                    .use_style(&STYLE)
                    .on_toggle(CommandMessages::ToggleFabricator {
                        fabricator_entity: module,
                    })
                    .bound_active(self.controls_enabled.clone())
                    .font(font, app),
            );
            out.push(
                ProgressBar::new(vec2(WIDTH - 8.0 * 2.0, 12.0))
                    .use_style(&STYLE)
                    .bind(progress.clone()),
            );
            out.push(Label::new(ready_text).font(font, app));
            out.push(Label::bound(countdown.clone()).font(font, app));
            out.push(
                Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Cancel")
                    .use_style(&STYLE)
                    .bound_active(self.controls_enabled.clone())
                    .on_click(CommandMessages::CancelActiveFabricator {
                        fabricator_entity: module,
                    }),
            );

            out.bindings.push(Binding::new({
                let current_et = self.current_et.clone();
                move |world: &World| {
                    let factory = world.get::<&Factory>(module).unwrap();
                    let job = factory.current_job.as_ref().unwrap();
                    progress.set(job.progress(&factory, current_et.get()) as f32);
                    let now = current_et.get();
                    let completion_et = job.completion_et(&factory, now);
                    let s = match completion_et {
                        Some(et) if now >= et => "DONE".into(),
                        Some(et) => format!("T- {}", (et - now).short_duration()),
                        None => String::from("T-"),
                    };
                    if *countdown.borrow() != s {
                        *countdown.borrow_mut() = s;
                    }
                }
            }))
        } else if let Some(part_id) = factory.pending_job {
            let part = self.parts.get(part_id).unwrap();

            let build_time_secs = part.cost.energy_joules / factory.power_watts;
            let completion = now + EphemerisTime::from_secs(build_time_secs as f64);

            out.push(Label::new(format!("Queued: {}", part.name)).font(font, app));
            out.push(Label::new(format!("Ready {}", completion.as_calendar())).font(font, app));
            out.push(Label::bound(countdown.clone()).font(font, app));
            out.push(
                Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Cancel")
                    .use_style(&STYLE)
                    .bound_active(self.controls_enabled.clone())
                    .on_click(CommandMessages::CancelQueuedFabricator {
                        fabricator_entity: module,
                    }),
            );

            out.bindings.push(Binding::new({
                let current_et = self.current_et.clone();
                move |_world: &World| {
                    let now = current_et.get();
                    let s = if now >= completion {
                        "DONE".into()
                    } else {
                        format!("T- {}", (completion - now).short_duration())
                    };
                    if *countdown.borrow() != s {
                        *countdown.borrow_mut() = s;
                    }
                }
            }))
        } else {
            out.push(
                Button::<CommandMessages>::text(vec2(WIDTH - 8.0 * 2.0, 30.0), "Build...")
                    .use_style_accented(&STYLE)
                    .bound_active(self.controls_enabled.clone())
                    .on_click(CommandMessages::OpenFabricator {
                        fabricator_entity: module,
                    }),
            );
        }

        out.bindings.push(Binding::new({
            let text = power_draw.clone();
            let enabled = enabled.clone();
            move |world: &World| {
                if let Ok(fab) = world.get::<&Factory>(module) {
                    enabled.set(fab.enabled);
                    let kw = if (fab.enabled && fab.current_job.is_some())
                        || fab.pending_job.is_some()
                    {
                        -fab.power_watts
                    } else {
                        0.0
                    } * Resource::Energy.presentation_scalars().1;
                    *text.borrow_mut() = format!("Power draw: {kw:-.2} kW")
                }
            }
        }));

        out
    }

    fn electrolyzer_section(&self, module: Entity, app: &App) -> Section {
        let font_small_bold = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font = app.renderer.get_font_id_from_name("font").unwrap();

        let mut out = Section::default();

        let text = Rc::new(RefCell::new(String::new()));
        let enabled = Rc::new(Cell::new(false));

        out.push(Label::new(String::from("ELECTROLYZER")).font(font_small_bold, app));
        out.push(
            Toggle::new("Enabled:")
                .bind(enabled.clone())
                .use_style(&STYLE)
                .on_toggle(CommandMessages::ToggleElectrolyzer {
                    electrolyzer_entity: module,
                })
                .bound_active(self.controls_enabled.clone())
                .font(font, app),
        );
        out.push(Label::bound(text.clone()).font(font, app));

        out.bindings.push(Binding::new({
            let text = text.clone();
            let enabled = enabled.clone();
            move |world: &World| {
                if let Ok(el) = world.get::<&Electrolyzer>(module) {
                    enabled.set(el.enabled);
                    let kw = if el.enabled { -el.power_watts } else { 0.0 }
                        * Resource::Energy.presentation_scalars().1;
                    *text.borrow_mut() = format!("Power draw: {kw:-.2} kW")
                }
            }
        }));

        out
    }

    fn gui_structure_key(&self) -> Option<(Entity, u32, u64, u64)> {
        let sel = self.selection.selected_entity()?;
        let gen = self
            .world
            .get::<&Station>(sel)
            .map(|s| s.modules_gen)
            .unwrap_or(0);

        Some((sel, gen, self.event_queue.version(), self.job_state_bits()))
    }

    fn job_state_bits(&self) -> u64 {
        let mut h = 0u64;
        for (e, f) in self.world.query::<&Factory>().iter() {
            let s = match (&f.current_job, f.pending_job) {
                (Some(_), _) => 2,
                (None, Some(_)) => 1,
                _ => 0,
            };
            h ^=
                (e.id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ s ^ ((f.enabled as u64) << 2);
        }
        for (e, el) in self.world.query::<&Electrolyzer>().iter() {
            h ^= (e.id() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (el.enabled as u64);
        }
        for (e, c) in self.world.query::<&Craft>().iter() {
            let s = match (&c.command, c.command_scheduled) {
                (Some(_), true) => 2,
                (Some(_), false) => 1,
                _ => 0,
            };
            h ^= (e.id() as u64).wrapping_mul(0x1656_67B1_9E37_79F9) ^ s;
        }
        h
    }

    fn commit_pending_builds(&self, now: EphemerisTime) {
        // Collect the factories
        let pending: Vec<(Entity, u64)> = self
            .world
            .query::<&Factory>()
            .iter()
            .filter_map(|(e, f)| f.pending_job.map(|id| (e, id)))
            .collect();

        for (fab, part_id) in pending {
            let station = self.world.get::<&Parent>(fab).unwrap().id;
            let cost = &self.parts.get(part_id).unwrap().cost;

            // Commit the parts subtraction
            {
                let mut inv = self.world.get::<&mut PartInventory>(station).unwrap();
                for (id, n) in &cost.parts {
                    for _ in 0..*n {
                        inv.take(*id).unwrap();
                    }
                }
            }

            // Commit the resources subtraction
            for (r, amount) in &cost.resources {
                take_resource(&self.world, station, *r, *amount, now);
            }

            // Commit at the old rate, before the job changes it.
            commit_station(&self.world, station, now);

            {
                let mut f = self.world.get::<&mut Factory>(fab).unwrap();
                f.start_job(part_id, now, cost.energy_joules).unwrap();
                f.pending_job = None;
            }

            // update for module ui
            self.world.get::<&mut Station>(station).unwrap().modules_gen += 1;
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

                replace_line_path(
                    &mut self.world,
                    &app.renderer,
                    craft,
                    Some((
                        WorldPosition {
                            pos: new_parent_world_pos, // center the orbit line path about the new parent
                        },
                        Parent { id: new_parent },
                        LinePathComponent::new(
                            new_craft_orbit
                                .generate_orbit_vertices(8192, new_parent_mu, Some(new_soi_radius))
                                .unwrap(),
                        ),
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
                replace_line_path(
                    &mut self.world,
                    &app.renderer,
                    craft,
                    Some((
                        WorldPosition {
                            pos: parent_world_pos,
                        },
                        Parent { id: parent },
                        LinePathComponent::new(
                            new_orbit
                                .generate_orbit_vertices(8192, parent_mu, soi_radius)
                                .unwrap(),
                        ),
                        AssociatedEntity { associate: craft },
                    )),
                );
                {
                    let mut craft_component = self.world.get::<&mut Craft>(craft).unwrap();
                    craft_component.burn(dv);
                }
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
                self.world.insert_one(craft, Landed { offset }).unwrap();
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
        for (station, _) in self.world.query::<&Station>().iter() {
            commit_station(&self.world, station, self.current_et.get());
        }
    }

    fn complete_due_jobs(&mut self, now: EphemerisTime, app: &App) {
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
            if def.fuel.is_some() {
                self.eject_craft(parent, &def, app);
            } else {
                let mut part_inventory = self.world.get::<&mut PartInventory>(parent).unwrap();
                part_inventory.add(part_id, 1);
            }

            // clear job so that factory becomes idle
            if let Ok(mut f) = self.world.get::<&mut Factory>(fab) {
                f.current_job = None;
            }
        }
    }

    fn eject_craft(&mut self, station: Entity, def: &PartDef, app: &App) {
        let now = self.current_et.get();
        let state = *self.world.get::<&State>(station).unwrap();
        let parent = *self.world.get::<&Parent>(station).unwrap();

        // Fuel it from the station's tanks, partially if that's all there is
        let fuel = def.fuel.unwrap();
        let want = fuel.max_fuel_mass_kg as f32;
        let (h2, _) = station_resource_totals(&self.world, station, Resource::Hydrogen, now);
        let (o2, _) = station_resource_totals(&self.world, station, Resource::Oxygen, now);

        // TODO: I'd rather put the fuels in the fuel struct itself, than have these ratios hardcoded
        const OF_RATIO: f32 = 5.5;
        let available = (h2 * (1.0 + OF_RATIO)).min(o2 * (1.0 + OF_RATIO) / OF_RATIO);
        let loaded = want.min(available);

        take_resource(
            &self.world,
            station,
            Resource::Hydrogen,
            loaded / (1.0 + OF_RATIO),
            now,
        );
        take_resource(
            &self.world,
            station,
            Resource::Oxygen,
            loaded * OF_RATIO / (1.0 + OF_RATIO),
            now,
        );

        let mut stage = def.instantiate_stage();
        stage.fuel_mass = loaded as f64;

        let craft = spawn_orbiting_craft(
            Payload {
                name: def.name.clone(),
                dry_mass: 0.0,
            },
            vec![stage],
            SceneObject {
                bvh_node_id: None,
                name: def.name.clone(),
            },
            parent,
            state,
            &mut self.world,
            &app.renderer,
            &mut self.bvh,
        );
        self.selection.crafts.push(craft);
    }

    fn crew_death(&self) -> Option<(Entity, Resource)> {
        const CRITICAL: [Resource; 3] = [Resource::Oxygen, Resource::Water, Resource::Energy];
        let now = self.current_et.get();
        for (station, s) in self.world.query::<&Station>().iter() {
            if s.num_crew == 0 {
                continue;
            }
            for r in CRITICAL {
                let (stored, _) = station_resource_totals(&self.world, station, r, now);
                let flow = station_resource_amount_flow(&self.world, station, r, false);
                // Under a second of supply counts as empty, so float rounding at the stop can't delay it
                if flow < 0.0 && stored / -flow < 1.0 {
                    return Some((station, r));
                }
            }
        }
        None
    }

    /// Updates planets based on their on-rails orbits around their parent bodies
    fn orbit_system(&mut self) {
        // Build parent -> children map
        let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();

        for (entity, (parent, _model)) in self.world.query::<(&Parent, &ModelComponent)>().iter() {
            children.entry(parent.id).or_default().push(entity);
        }

        // Collect all entities with WorldPosition
        let mut has_parent = HashMap::new();
        for (entity, parent) in self.world.query::<&Parent>().iter() {
            has_parent.insert(entity, parent.id);
        }

        // Find roots (entities without parent)
        let mut roots = Vec::new();
        for (entity, _) in self.world.query::<(&WorldPosition, &Body)>().iter() {
            if !has_parent.contains_key(&entity) {
                roots.push(entity);
            }
        }

        let et = self.current_et.get();

        // Kick off from roots
        for root in roots {
            let mu = { self.world.get::<&mut Body>(root).unwrap().mu };
            let root_pos = vec3(0.0, 0.0, 0.0);
            self.propagate(&children, root, root_pos, mu, et);
        }
    }

    fn propagate(
        &mut self,
        children: &HashMap<Entity, Vec<Entity>>,
        entity: Entity,
        parent_pos: DVec3,
        parent_mu: f64,
        t: EphemerisTime,
    ) {
        let mut world_pos = self.world.get::<&mut WorldPosition>(entity).unwrap();

        let local_offset = if let Ok(orbit) = self.world.get::<&State>(entity) {
            match orbit.propagate(t, parent_mu) {
                Ok(s) => s.r,
                Err(_) => return,
            }
        } else {
            vec3(0.0, 0.0, 0.0)
        };

        let new_world = parent_pos + local_offset;
        world_pos.pos = new_world;

        drop(world_pos);

        if let Some(kids) = children.get(&entity) {
            for &child in kids {
                let mu = { self.world.get::<&mut Body>(entity).unwrap().mu };
                self.propagate(children, child, new_world, mu, t);
            }
        }
    }

    // Updates craft to be on the surface of their planet
    fn landed_system(&mut self) {
        let mut pos_map = HashMap::new();
        for (entity, (world_pos, _body)) in self.world.query::<(&WorldPosition, &Body)>().iter() {
            pos_map.insert(entity, world_pos.pos);
        }

        for (_entity, (world_pos, parent, landed)) in
            self.world
                .query_mut::<(&mut WorldPosition, &Parent, &Landed)>()
        {
            let parent_pos = pos_map.get(&parent.id).unwrap();
            world_pos.pos = parent_pos + landed.offset;
        }
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
        for (fab, (_, _, f)) in self
            .world
            .query::<(&StationModule, &Parent, &Factory)>()
            .iter()
        {
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
        for (entity, (_, scene_obj)) in self.world.query::<(&Station, &SceneObject)>().iter() {
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
                        subject: scene_obj.name.clone(),
                        detail: format!("{} Depleted", resource.long_name()),
                    })
                } else {
                    marks.push(TimelineMark {
                        t: et,
                        kind: MarkKind::Good,
                        subject: scene_obj.name.clone(),
                        detail: format!("{} Filled", resource.long_name()),
                    })
                }
            }
        }

        // Add projected mission burns (burns, SOI crossings, etc)
        for (_, (craft, scene_obj)) in self.world.query::<(&Craft, &SceneObject)>().iter() {
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
                    subject: scene_obj.name.clone(),
                    detail: burn.desc.to_string(),
                });
            }
            for (label, et) in command.transition_schedule() {
                marks.push(TimelineMark {
                    t: et,
                    kind: MarkKind::SoiChange,
                    subject: scene_obj.name.clone(),
                    detail: label.to_string(),
                });
            }
        }

        marks
    }

    fn next_station_limit(&self, now: EphemerisTime) -> Option<EphemerisTime> {
        let mut limits = vec![];
        for (entity, _) in self.world.query::<&Station>().iter() {
            limits.extend(next_reservoir_limits(
                &self.world,
                entity,
                &self.parts,
                now,
                true,
            ));
        }

        limits.first().map(|(et, _, _)| *et)
    }

    fn craft_name_from_event(&self, event: &Event) -> (String, String) {
        match event {
            Event::SoiChange { craft, desc, .. } | Event::Burn { craft, desc, .. } => {
                let scene_obj = self.world.get::<&SceneObject>(*craft).unwrap();
                (scene_obj.name.clone(), desc.to_string())
            }

            Event::Launch { craft } | Event::Land { craft } => {
                let scene_obj = self.world.get::<&SceneObject>(*craft).unwrap();
                (scene_obj.name.clone(), "???".to_string())
            }

            Event::FactoryComplete { craft, part_id } => {
                let parent = self.world.get::<&Parent>(*craft).unwrap().id;
                let scene_obj = self.world.get::<&SceneObject>(parent).unwrap();
                let part_def = self.parts.get(*part_id).map_or("???", |p| p.name.as_str());
                (scene_obj.name.clone(), part_def.to_string())
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
        let closest_body = self
            .get_ancestor(closest_body.unwrap())
            .unwrap_or(self.selection.bodies[0]);
        let closest_planet = self.get_ancestor(closest_body).unwrap_or(closest_body);
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
            let assoc_planet = self
                .get_ancestor(assoc_entity)
                .unwrap_or(self.selection.bodies[0]);

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

    fn get_ancestor(&self, entity: Entity) -> Option<Entity> {
        let mut child = entity;
        loop {
            let parent = self.world.get::<&Parent>(child).ok()?; // if sun, this will return None (sun has no parent)
            let parent_body = self.world.get::<&Body>(parent.id).ok()?;
            if parent_body.mu == SUN_MU {
                return Some(child);
            }
            child = parent.id;
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

    fn render_dots(&mut self, app: &App) {
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
        let turn_key = Some((
            app.window_size,
            self.marks_key,
            self.pause_reasons.first().map(|m| m.t),
        ));
        if turn_key != self.turn_gui_built_for {
            self.turn_gui_built_for = turn_key;
            self.turn_gui = self.rebuild_turn_gui(app);
            self.gui_built_for = None;
        }

        let key = self.gui_structure_key();
        if key != self.gui_built_for {
            self.gui_built_for = key;
            let (gui, bindings) = self.rebuild_gui(app);
            self.gui = gui;
            self.gui_bindings = bindings;
        }

        for binding in &self.gui_bindings {
            binding.sync(&self.world);
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
