//! This module is responsible for defining the gameplay scene.

use std::{cell::Cell, collections::HashMap, rc::Rc};

use apricot::{
    app::{App, Scene},
    bvh::BVH,
};
use hecs::{Entity, World};
use sdl2::keyboard::Scancode;

use crate::{
    astro::{epoch::EphemerisTime, state::State, units::SUN_MU},
    container,
    generation::lexicon::Lexicon,
    hud::{
        fabricator::{FabricatorAction, FabricatorUi},
        footer::{Footer, FooterView, TurnMessages},
        game_over::GameOverUi,
        maneuver::ManeuverModal,
        panel::{self, panel_structure_bits, CommandMessages, PanelCtx},
        timeline::{self},
        transfer::{TransferResult, TransferUi},
        Binding,
    },
    render::{
        assets::load_assets,
        camera::{focus_point, CameraRig},
        orbit_lines::{redraw_orbit, replace_line_path, style_orbit_lines},
        picking::Picker,
        scene::{attach_craft_model, SceneRenderer},
    },
    sim::{
        bodies::{Body, Category},
        docking::{Docking, PortHost},
        hierarchy::{Named, ParentBody},
        industry::Factory,
        life_support::Station,
        parts::{id_hash, PartInventory, PartRegistry},
        propulsion::spawn_craft,
        resources::{Electrolyzer, Resource, ResourceStore, SolarPanel},
        Sim, SimEffect,
    },
    ui::anchor::{Anchor, AnchorPoint},
};

use crate::{
    components::body::{spawn_body, SceneObject},
    generation::solar_system_gen::{self},
    ui::{
        container::Container,
        widget::{recv_msgs, Widget},
    },
};

/// Struct that contains info about the game state
pub struct Gameplay {
    sim: Sim,

    selection: SelectionState,

    rig: CameraRig,
    scene: SceneRenderer,
    picker: Picker,

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
    pub changed_at: f64,
}

impl SelectionState {
    pub fn new(crafts: Vec<Entity>, bodies: Vec<Entity>, buildings: Vec<Entity>) -> Self {
        Self {
            crafts,
            bodies,
            buildings,
            selected: None,
            kind: SelectionKind::Body,
            changed_at: 0.0,
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

            self.changed_at = app_seconds;
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

        self.changed_at = app_seconds;
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

        self.changed_at = app_seconds;
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
            self.sim.queue_build(fabricator, part_id);
        }

        let now = self.sim.clock().now();

        if let Some((craft, command)) = self.maneuver_ui.update(now, self.sim.world(), app) {
            self.sim.assign_command(craft, command);
        }

        if let Some(TransferResult { from, to }) =
            self.transfer_ui.update(now, self.sim.world(), app)
        {
            self.sim.transfer(from, to);
            self.transfer_ui.rebuild(now, self.sim.world(), app);
        }

        if !modal_open {
            // Handle all the messages from UI
            for msg in recv_msgs(app, &mut self.gui) {
                match msg {
                    CommandMessages::OpenFabricator { fabricator_entity } => {
                        self.fabricator_ui.show(
                            self.sim.world(),
                            fabricator_entity,
                            self.sim.parts(),
                            self.sim.clock().now(),
                            app,
                        );
                    }
                    CommandMessages::CancelQueuedFabricator { fabricator_entity } => {
                        self.sim.cancel_queued_build(fabricator_entity);
                    }
                    CommandMessages::CancelActiveFabricator { fabricator_entity } => {
                        self.sim.cancel_active_build(fabricator_entity);
                    }
                    CommandMessages::ToggleFabricator { fabricator_entity } => {
                        self.sim.toggle_fabricator(fabricator_entity);
                    }
                    CommandMessages::CancelCommand { craft } => {
                        self.sim.cancel_command(craft);
                    }
                    CommandMessages::ToggleElectrolyzer {
                        electrolyzer_entity,
                    } => {
                        self.sim.toggle_electrolyzer(electrolyzer_entity);
                    }
                    CommandMessages::ToggleMiner { miner_entity } => {
                        self.sim.toggle_miner(miner_entity);
                    }
                    CommandMessages::Undock { entity } => {
                        self.sim.undock(entity);
                    }
                    CommandMessages::SelectEntity { entity } => {
                        self.selection.set_selected(entity, app.seconds as f64);
                    }
                    CommandMessages::OpenManeuver { craft } => {
                        self.maneuver_ui
                            .show(craft, self.sim.clock().now(), self.sim.world(), app);
                    }
                    CommandMessages::OpenTransfer { craft } => {
                        self.transfer_ui
                            .show(craft, self.sim.clock().now(), self.sim.world(), app);
                    }
                }
            }

            for msg in self.footer.update(app) {
                match msg {
                    TurnMessages::TogglePlay => {
                        self.sim.toggle_play();
                        if !self.sim.clock().paused() {
                            self.footer.resumed();
                        }
                    }
                    TurnMessages::SpeedUp => self.sim.speed_up(),
                    TurnMessages::SlowDown => self.sim.slow_down(),
                }
            }
        }

        self.sim.step(1.0 / 60.0);
        self.apply_sim_effects(app);

        // Update GUI stuff
        self.controls_enabled.set(self.sim.clock().paused());

        if !modal_open {
            self.handle_tab(app);
            self.rig
                .orbit_controls(app, self.selected_body_radius().unwrap_or(0.0));
            let selected = self.selection.selected_entity();
            if let Some(clicked) = self
                .picker
                .update(self.sim.world(), &self.rig, selected, app)
            {
                self.selection.set_selected(clicked, app.seconds as f64);
            }
        }

        let focus = focus_point(self.sim.world(), self.selection.selected_entity());
        self.rig.update(focus, self.selection.changed_at, app);

        let selected = self.selection.selected_entity();
        self.picker
            .sync_selected_tile(self.sim.world(), selected, &app.renderer);

        let highlighted = selected.filter(|_| !self.rig.is_animating(app.seconds as f64));
        let camera_pos = self.rig.world_pos();
        let sun = self.selection.bodies[0];
        let now = self.sim.clock().now();
        style_orbit_lines(self.sim.world_mut(), camera_pos, highlighted, sun, now);

        self.scene
            .sync_models(self.sim.world_mut(), camera_pos, app);

        let marks = timeline::build_marks(&self.sim);
        self.footer.set_marks(marks);
        self.sync_panel(app);

        // Delete anything we want deleted
        app.renderer.flush_deletion_queue();
    }

    /// Render the scene to the screen when time allows
    fn render(&mut self, app: &App) {
        self.rig.sync_aspect(app);
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        app.renderer.set_font(font);

        self.scene.render(self.sim.world_mut(), &self.rig, app);
        let selected = self.selection.selected_entity();
        self.picker
            .render(self.sim.world(), &self.rig, selected, app);

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
    pub fn new(app: &App) -> Self {
        let tile_sets = load_assets(&app.renderer);
        let mut world = World::new();
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
                Some(ParentBody { id: sun_entity }),
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
                    Some(ParentBody { id: planet_entity }),
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
            .instantiate_craft();

        let station = spawn_craft(
            station_payload,
            Named {
                name: String::from("Station"),
            },
            ParentBody { id: station_parent },
            &mut world,
        );
        attach_craft_model(&mut world, &app.renderer, &mut bvh, station);
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
        redraw_orbit(&mut world, &app.renderer, station, None);

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
        ));
        world.spawn((
            Docking {
                host: station,
                own_port: 0,
                host_port: 1,
            },
            SolarPanel { rated_w: 100_000.0 },
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
        ));
        crafts.push(station);

        let mut selection = SelectionState::new(crafts, bodies, buildings);
        selection.set_selected(station, app.seconds as f64 - 1.0);

        let mut retval = Self {
            sim: Sim::new(world, parts),

            rig: CameraRig::new(),
            scene: SceneRenderer::new(bvh),
            picker: Picker::default(),

            selection,

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
        };

        retval.sync_panel(app);

        retval
    }

    fn handle_tab(&mut self, app: &App) {
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
    }

    fn gui_structure_key(&self) -> Option<(Entity, u32, u64, u64)> {
        let sel = self.selection.selected_entity()?;
        let gen = self
            .sim
            .world()
            .get::<&PortHost>(sel)
            .map(|s| s.dock_gen)
            .unwrap_or(0);

        Some((
            sel,
            gen,
            self.sim.events().version(),
            panel_structure_bits(self.sim.world()),
        ))
    }

    fn apply_sim_effects(&mut self, app: &App) {
        for effect in self.sim.drain_effects() {
            match effect {
                SimEffect::OrbitChanged { craft, soi_radius } => {
                    redraw_orbit(self.sim.world_mut(), &app.renderer, craft, soi_radius);
                }
                SimEffect::OrbitCleared { craft } => {
                    replace_line_path(self.sim.world_mut(), &app.renderer, craft, None);
                    self.sim.world_mut().remove_one::<State>(craft).ok();
                }
                SimEffect::CraftSpawned { craft } => {
                    attach_craft_model(
                        self.sim.world_mut(),
                        &app.renderer,
                        self.scene.bvh_mut(),
                        craft,
                    );
                    self.selection.crafts.push(craft);
                }
                SimEffect::Focus { entity } => {
                    self.selection.set_selected(entity, app.seconds as f64);
                }
                SimEffect::Stopped { at } => self.footer.stopped_at(at),
                SimEffect::CrewLost { station, cause } => {
                    let name = self
                        .sim
                        .world()
                        .get::<&Named>(station)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    self.game_over_ui
                        .show(&name, cause, self.sim.clock().now(), app);
                }
            }
        }
    }

    fn selected_body_radius(&self) -> Option<f64> {
        let entity = self.selection.selected_entity()?;
        let mut q = self.sim.world().query_one::<&Body>(entity).ok()?;
        let body = q.get()?;
        Some(body.body_radius)
    }

    fn sync_panel(&mut self, app: &App) {
        let now = self.sim.clock().now();

        let footer_view = FooterView {
            now,
            paused: self.sim.clock().paused(),
            speed_label: self.sim.clock().rate_label(),
            can_speed_up: self.sim.clock().can_speed_up(),
            can_slow_down: self.sim.clock().can_slow_down(),
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
                world: self.sim.world(),
                parts: self.sim.parts(),
                now,
                controls_enabled: &self.controls_enabled,
            };
            let (gui, bindings) =
                panel::build(&ctx, self.selection.selected_entity(), self.footer.height());
            self.gui = gui;
            self.gui_bindings = bindings;
        }

        for binding in &self.gui_bindings {
            binding.sync(self.sim.world(), now);
        }
    }
}
