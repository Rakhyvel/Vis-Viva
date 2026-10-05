//! This module is responsible for defining the gameplay scene.

use std::{cell::Cell, rc::Rc};

use apricot::{
    app::{App, Scene},
    bvh::BVH,
};
use hecs::Entity;
use sdl2::keyboard::Scancode;

use crate::{
    astro::state::State,
    container,
    generation::new_game::{new_game, NewGame},
    hud::{
        emergency::{emergency_lines, EmergencyBanner},
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
        orbit_lines::{redraw_orbit, replace_line_path, spawn_body_orbit_line, style_orbit_lines},
        picking::Picker,
        scene::{attach_body_model, attach_craft_model, SceneRenderer},
    },
    scenes::selection::SelectionState,
    sim::{
        bodies::Body,
        docking::PortHost,
        hierarchy::{Named, ParentBody},
        parts::PartRegistry,
        Sim, SimEffect,
    },
    ui::anchor::{Anchor, AnchorPoint},
};

use crate::ui::{
    container::Container,
    widget::{recv_msgs, Widget},
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
    emergency_banner: EmergencyBanner,

    /// Buttons in the side panel are only clickable while paused
    controls_enabled: Rc<Cell<bool>>,
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
        self.emergency_banner
            .sync(app, emergency_lines(self.sim.world(), now));

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
        self.emergency_banner.render(app);
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
        let parts = PartRegistry::load_from_dir("res/parts");
        let NewGame {
            mut world,
            bodies,
            crafts,
            station,
        } = new_game(&parts, &tile_sets);

        // Give the generated world its models and orbit lines
        let mut bvh = BVH::<Entity>::new();
        for &body in &bodies {
            attach_body_model(&mut world, &app.renderer, &mut bvh, body);
            if world.get::<&ParentBody>(body).is_ok() {
                // skip the sun
                spawn_body_orbit_line(&mut world, body);
            }
        }

        for &craft in &crafts {
            attach_craft_model(&mut world, &app.renderer, &mut bvh, craft);
            redraw_orbit(&mut world, &app.renderer, craft, None);
        }

        let mut selection = SelectionState::new(crafts, bodies, vec![]);
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
            emergency_banner: EmergencyBanner::new(),

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
