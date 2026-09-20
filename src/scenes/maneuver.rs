use std::{
    cell::{Cell, RefCell},
    f64::consts::PI,
    rc::Rc,
};

use crate::{
    astro::{
        departure::{sweep_window, SweepWindow, TransferObjective},
        epoch::EphemerisTime,
        escape::{plan_escape, EscapePlan},
        landing::{plan_landing, LandingPlan},
        launch::{plan_launch, LaunchPlan},
        porkchop::Porkchop,
        rendezvous::{plan_rendezvous_at, rendezvous_porkchop, RendezvousPlan},
        state::State,
        transfer::{
            flyby_porkchop, plan_flyby_at, plan_transfer_at, transfer_porkchop, FlybyPlan,
            TransferPlan,
        },
        units::{G, KM_PER_EARTH_RADIUS, METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR},
    },
    components::{
        body::{Body, Parent, SceneObject},
        craft::{Command, Craft, Landed},
        station::stored_mass_kg,
    },
    ui::{
        container::Container,
        dropdown::Dropdown,
        hrule::HRule,
        oklch::oklch,
        porkchop_picker::{PlotAxes, PorkchopPicker},
        slider::Slider,
        style::STYLE,
    },
};
use apricot::{app::App, font::FontId, render_core::TextureId};
use hecs::{Entity, World};
use nalgebra_glm::{vec2, Vec4};

use crate::{
    container,
    ui::{
        button::Button,
        container::{Align, Flow},
        label::Label,
        modal::Modal,
        widget::{recv_msgs, Widget},
    },
};

const WIDTH: f32 = 280.0;

const DEPART_STEPS: usize = 46;
const TOF_STEPS: usize = 34;

pub struct ManeuverModal {
    modal: Modal<ManeuverMessages>,
    craft: Option<Entity>,

    selected_kind: Option<ManeuverKind>,
    selected_destination: Option<Entity>,
    selected_date: EphemerisTime,
    computed_plan: Option<ManeuverResult>,

    result_dv_text: Rc<RefCell<String>>,
    result_depart_text: Rc<RefCell<String>>,
    result_date_text: Rc<RefCell<String>>,
    inclination_text: Rc<RefCell<String>>,
    dv_color: Rc<RefCell<Vec4>>,
    can_confirm: Rc<Cell<bool>>,
    theta: Rc<Cell<f32>>,
    axes: Rc<Cell<Option<PlotAxes>>>,

    budget: f64,
    window: Option<SweepWindow>,
    porkchop: Option<Porkchop>,
    porkchop_texture_id: TextureId,
    selected_cell: Rc<Cell<(usize, usize)>>,
    last_cell: (usize, usize),
    optimum: Rc<Cell<(usize, usize)>>,
    last_theta: f32,
}

#[derive(Clone, Debug)]
enum ManeuverMessages {
    SelectKind(ManeuverKind),
    SelectDestination(Entity),
    ShiftPorkchopLeft,
    ShiftPorkchopRight,
    ZoomPorkchopIn,
    ZoomPorkchopOut,
    Confirm,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ManeuverKind {
    Transfer,
    Flyby,
    Escape,
    Land,
    Launch,
    Rendezvous,
    Dock,
}

#[derive(PartialEq, Clone, Copy)]
enum TargetKind {
    Body,
    Craft,
    CoLocatedCraft,
}

struct ManeuverOptions {
    is_landed: bool,
    is_orbiting: bool,
    parent_is_solid: bool,
    can_escape: bool,
    bodies: Vec<(Entity, String)>,
    crafts: Vec<(Entity, String)>,
    /// Vec of craft that have the same position and velocity
    co_located: Vec<(Entity, String)>,
}

impl ManeuverKind {
    pub fn all() -> &'static [ManeuverKind] {
        &[
            ManeuverKind::Transfer,
            ManeuverKind::Flyby,
            ManeuverKind::Escape,
            ManeuverKind::Land,
            ManeuverKind::Launch,
            ManeuverKind::Rendezvous,
            ManeuverKind::Dock,
        ]
    }

    pub fn available(&self, o: &ManeuverOptions) -> bool {
        // Idea here is to only hide maneuver kinds that are physically impossible, no matter how good your
        // craft is. Transfers that cost a million km/s of dv are shown, but locked out. Landing with a low
        // TWR is shown, but locked out. You could make a craft that's super good and fix both of those.
        // There is no craft you can make to land on a gas giant, so it's hidden.
        match self {
            ManeuverKind::Transfer => o.is_orbiting && !o.bodies.is_empty(),
            ManeuverKind::Flyby => o.is_orbiting && !o.bodies.is_empty(),
            ManeuverKind::Escape => o.is_orbiting && o.can_escape,
            ManeuverKind::Land => o.is_orbiting && o.parent_is_solid,
            ManeuverKind::Launch => o.is_landed,
            ManeuverKind::Rendezvous => o.is_orbiting && !o.crafts.is_empty(),
            ManeuverKind::Dock => o.is_orbiting && !o.co_located.is_empty(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ManeuverKind::Transfer => "Transfer",
            ManeuverKind::Flyby => "Flyby",
            ManeuverKind::Escape => "Escape",
            ManeuverKind::Land => "Land",
            ManeuverKind::Launch => "Launch",
            ManeuverKind::Rendezvous => "Rendezvous",
            ManeuverKind::Dock => "Dock",
        }
    }

    pub fn target_kind(&self) -> Option<TargetKind> {
        match self {
            ManeuverKind::Transfer | ManeuverKind::Flyby => Some(TargetKind::Body),
            ManeuverKind::Rendezvous => Some(TargetKind::Craft),
            ManeuverKind::Dock => Some(TargetKind::CoLocatedCraft),
            ManeuverKind::Escape | ManeuverKind::Land | ManeuverKind::Launch => None,
        }
    }
}

pub enum ManeuverResult {
    Transfer {
        to: Entity,
        plan: TransferPlan,
    },
    Flyby {
        to: Entity,
        from: Entity,
        plan: FlybyPlan,
    },
    Rendezvous {
        with: Entity,
        plan: RendezvousPlan,
    },
    Escape {
        to: Entity,
        from: Entity,
        plan: EscapePlan,
    },
    Land {
        on: Entity,
        plan: LandingPlan,
    },
    Launch {
        from: Entity,
        plan: LaunchPlan,
    },
    Dock {
        with: Entity,
        depart_et: EphemerisTime,
        arrive_et: EphemerisTime,
    },
}

impl ManeuverResult {
    pub fn total_dv(&self) -> f64 {
        match self {
            ManeuverResult::Transfer { plan, .. } => plan.transfer_dv + plan.circ_dv,
            ManeuverResult::Flyby { plan, .. } => plan.transfer_dv,
            ManeuverResult::Rendezvous { plan, .. } => plan.transfer_dv + plan.brake_dv,
            ManeuverResult::Escape { plan, .. } => plan.escape_dv,
            ManeuverResult::Land { plan, .. } => plan.deorbit_dv + plan.landing_dv,
            ManeuverResult::Launch { plan, .. } => plan.launch_dv + plan.circ_dv,
            ManeuverResult::Dock { .. } => 0.0,
        }
    }

    pub fn departure_et(&self) -> EphemerisTime {
        match self {
            ManeuverResult::Transfer { plan, .. } => plan.transfer_state.t,
            ManeuverResult::Flyby { plan, .. } => plan.transfer_state.t,
            ManeuverResult::Rendezvous { plan, .. } => plan.transfer_state.t,
            ManeuverResult::Escape { plan, .. } => plan.escape_burn.t,
            ManeuverResult::Land { plan, .. } => plan.deorbit_burn.t,
            ManeuverResult::Launch { plan, .. } => plan.launch_burn.t,
            ManeuverResult::Dock { depart_et, .. } => *depart_et,
        }
    }

    pub fn arrival_et(&self) -> EphemerisTime {
        match self {
            ManeuverResult::Transfer { plan, .. } => plan.circ_state.t,
            ManeuverResult::Flyby { plan, .. } => plan.flyby_state.t,
            ManeuverResult::Rendezvous { plan, .. } => plan.rendezvous_state.t,
            ManeuverResult::Escape { plan, .. } => plan.exit_state.t,
            ManeuverResult::Land { plan, .. } => plan.landing_burn.t,
            ManeuverResult::Launch { plan, .. } => plan.circ_burn.t,
            ManeuverResult::Dock { arrive_et, .. } => *arrive_et,
        }
    }

    pub fn can_afford(&self, craft_dv: f64) -> bool {
        self.total_dv() <= craft_dv
    }

    pub fn into_command(self) -> Command {
        match self {
            ManeuverResult::Transfer { to, plan } => Command::Transfer { to, plan },
            ManeuverResult::Flyby { to, from, plan } => Command::Flyby { to, from, plan },
            ManeuverResult::Rendezvous { with, plan } => Command::Rendezvous { with, plan },
            ManeuverResult::Escape { to, from, plan } => Command::Escape { to, from, plan },
            ManeuverResult::Land { on, plan } => Command::Land { on, plan },
            ManeuverResult::Launch { from, plan } => Command::Launch { from, plan },
            ManeuverResult::Dock {
                with, arrive_et, ..
            } => Command::Dock { with, arrive_et },
        }
    }
}

impl ManeuverModal {
    pub fn new(app: &App) -> Self {
        Self {
            modal: Modal::new(Box::new(container![])),
            craft: None,

            selected_kind: None,
            selected_destination: None,
            selected_date: EphemerisTime::epoch(),
            computed_plan: None,

            result_depart_text: Rc::new(RefCell::new(String::new())),
            result_date_text: Rc::new(RefCell::new(String::new())),
            result_dv_text: Rc::new(RefCell::new(String::new())),
            inclination_text: Rc::new(RefCell::new(String::new())),
            dv_color: Rc::new(RefCell::new(Vec4::zeros())),
            can_confirm: Rc::new(Cell::new(false)),
            theta: Rc::new(Cell::new(0.0)),
            axes: Rc::new(Cell::new(None)),

            budget: 0.0,
            window: None,
            porkchop: None,
            porkchop_texture_id: app.renderer.create_texture_rgba(1, 1, &[0, 0, 0, 0]),
            selected_cell: Rc::new(Cell::new((0, 0))),
            last_cell: (0, 0),
            optimum: Rc::new(Cell::new((0, 0))),
            last_theta: 0.0,
        }
    }

    pub fn show(&mut self, craft: Entity, current_et: EphemerisTime, world: &World, app: &App) {
        self.craft = Some(craft);

        self.selected_kind = None;
        self.selected_destination = None;
        self.selected_date = current_et;
        self.computed_plan = None;

        self.rebuild(current_et, world, app);
        self.modal.set_shown(true);
    }

    pub fn is_shown(&self) -> bool {
        self.modal.is_shown()
    }

    pub fn update(
        &mut self,
        current_et: EphemerisTime,
        world: &World,
        app: &App,
    ) -> Option<ManeuverResult> {
        let craft = self.craft?;
        for msg in recv_msgs(app, &mut self.modal) {
            match msg {
                ManeuverMessages::SelectKind(maneuver_kind) => {
                    if self
                        .selected_kind
                        .is_some_and(|k| k.target_kind() != maneuver_kind.target_kind())
                    {
                        // Wipe the selected destination if it's not the same kind, or it'll mess stuff up later
                        self.selected_destination = None;
                    }
                    self.porkchop = None;
                    self.selected_kind = Some(maneuver_kind);
                    self.refresh_window(craft, current_et, world);
                    self.refresh_chop(craft, current_et, world, app, true);
                    self.rebuild(current_et, world, app)
                }
                ManeuverMessages::SelectDestination(entity) => {
                    self.selected_destination = Some(entity);
                    self.refresh_window(craft, current_et, world);
                    self.refresh_chop(craft, current_et, world, app, true);
                }
                ManeuverMessages::ShiftPorkchopLeft => {
                    if let Some(window) = &mut self.window {
                        let half = EphemerisTime::from_years(window.sweep * 0.5);
                        window.start = (window.start - half).max(current_et);
                    }
                    self.refresh_chop(craft, current_et, world, app, true);
                }
                ManeuverMessages::ShiftPorkchopRight => {
                    if let Some(window) = &mut self.window {
                        window.start += EphemerisTime::from_years(window.sweep * 0.5);
                    }
                    self.refresh_chop(craft, current_et, world, app, true);
                }
                ManeuverMessages::ZoomPorkchopIn => {
                    self.zoom_porkchop(0.5, craft, current_et, world, app)
                }
                ManeuverMessages::ZoomPorkchopOut => {
                    self.zoom_porkchop(2.0, craft, current_et, world, app)
                }
                ManeuverMessages::Close => {
                    self.porkchop = None;
                    self.modal.set_shown(false);
                }
                ManeuverMessages::Confirm => {
                    self.porkchop = None;
                    self.modal.set_shown(false);
                    return self.computed_plan.take();
                }
            }
        }

        let cell = self.selected_cell.get();
        if cell != self.last_cell {
            self.last_cell = cell;
            self.computed_plan = self.plan_from_selection(craft, current_et, world);
        }

        let theta = self.theta.get();
        if theta != self.last_theta && !app.mouse_left_dragging {
            self.last_theta = theta;
            self.refresh_chop(craft, current_et, world, app, true);
        }

        self.sync_labels(world);

        None
    }

    fn refresh_window(&mut self, craft: Entity, current_et: EphemerisTime, world: &World) {
        self.window = None;
        let Some(target) = self.selected_destination else {
            return;
        };
        let Ok(craft_state) = world.get::<&State>(craft) else {
            return;
        };
        let Ok(target_state) = world.get::<&State>(target) else {
            return;
        };
        let Ok(parent) = world.get::<&Parent>(craft) else {
            return;
        };
        let Ok(parent_body) = world.get::<&Body>(parent.id) else {
            return;
        };

        self.window = sweep_window(
            &craft_state,
            &target_state,
            G * parent_body.mass(),
            current_et,
        )
        .ok()
    }

    fn refresh_chop(
        &mut self,
        craft: Entity,
        current_et: EphemerisTime,
        world: &World,
        app: &App,
        update_selected: bool,
    ) {
        self.porkchop = self.compute_porkchop(craft, world);

        if let Some(chop) = &self.porkchop {
            let cargo_kg = stored_mass_kg(world, craft, current_et);
            self.budget = world
                .get::<&Craft>(craft)
                .map(|c| c.total_remaining_dv(cargo_kg))
                .unwrap_or(0.0)
                / METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR;
            let bytes = self.porkchop_rgba(chop);

            app.renderer.update_texture_rgba(
                self.porkchop_texture_id,
                chop.depart_steps as u32,
                chop.tof_steps as u32,
                &bytes,
            );

            self.axes.set(Some(PlotAxes {
                depart_start: chop.current_et,
                depart_step: chop.step,
                tof_min: chop.tof_min,
                tof_max: chop.tof_max,
            }));

            if let Some((i, j, _)) = chop.best(&TransferObjective::MinFuel) {
                self.optimum.set((i, j));
                if update_selected {
                    self.selected_cell.set((i, j));
                    self.last_cell = (i, j);
                }
            } else {
                self.selected_cell.set((0, 0));
                self.last_cell = (0, 0);
                self.optimum.set((0, 0));
            }
        } else {
            self.axes.set(None);
            app.renderer
                .update_texture_rgba(self.porkchop_texture_id, 1, 1, &[0, 0, 0, 0]);
        }

        self.computed_plan = self.plan_from_selection(craft, current_et, world);
    }

    fn zoom_porkchop(
        &mut self,
        factor: f64,
        craft: Entity,
        current_et: EphemerisTime,
        world: &World,
        app: &App,
    ) {
        let Some(window) = &mut self.window else {
            return;
        };
        // zoom about the puck
        let focus = self
            .porkchop
            .as_ref()
            .map(|c| c.depart_at(self.selected_cell.get().0))
            .unwrap_or(window.start);

        window.sweep = (window.sweep * factor).min(window.full);
        let half = EphemerisTime::from_years(window.sweep * 0.5);
        window.start = (focus - half).max(current_et);

        self.refresh_chop(craft, current_et, world, app, false);
    }

    fn sync_labels(&mut self, world: &World) {
        let dv = self
            .computed_plan
            .as_ref()
            .map_or_else(|| String::from("-"), |p| format!("{:.0} m/s", p.total_dv()));
        if *self.result_dv_text.borrow() != dv {
            *self.result_dv_text.borrow_mut() = dv;
        }

        let depart_date = self
            .computed_plan
            .as_ref()
            .map_or_else(|| String::from("-"), |p| p.departure_et().as_calendar());
        if *self.result_depart_text.borrow() != depart_date {
            *self.result_depart_text.borrow_mut() = depart_date;
        }

        let arrival_date = self
            .computed_plan
            .as_ref()
            .map_or_else(|| String::from("-"), |p| p.arrival_et().as_calendar());
        if *self.result_date_text.borrow() != arrival_date {
            *self.result_date_text.borrow_mut() = arrival_date;
        }

        let inclination = format!(
            "Inclination: {:.1} deg", // TODO: This is really the B-plane clock angle
            (self.theta.get() as f64 * 2.0 * PI).to_degrees()
        );
        if *self.inclination_text.borrow() != inclination {
            *self.inclination_text.borrow_mut() = inclination;
        }

        let cargo_kg: f64 = self.computed_plan.as_ref().map_or_else(
            || 0.0,
            |p| stored_mass_kg(world, self.craft.unwrap(), p.departure_et()),
        );
        let craft_dv = world
            .get::<&Craft>(self.craft.unwrap())
            .unwrap()
            .total_remaining_dv(cargo_kg);

        let can_afford_plan = self
            .computed_plan
            .as_ref()
            .map(|plan| plan.can_afford(craft_dv))
            .unwrap_or(false);

        self.can_confirm.set(can_afford_plan);

        let color = match &self.computed_plan {
            None => STYLE.text,
            Some(_) if can_afford_plan => STYLE.positive,
            Some(_) => STYLE.negative,
        };
        if *self.dv_color.borrow() != color {
            *self.dv_color.borrow_mut() = color
        }
    }

    pub fn render(&self, app: &App) {
        self.modal.render(app);
    }

    pub fn rebuild(&mut self, current_et: EphemerisTime, world: &World, app: &App) {
        let font = app.renderer.get_font_id_from_name("font").unwrap();
        let font_small_bold: FontId = app
            .renderer
            .get_font_id_from_name("font-small-bold")
            .unwrap();
        let font_big: FontId = app.renderer.get_font_id_from_name("font-big").unwrap();

        let mut sections: Vec<Box<dyn Widget<ManeuverMessages>>> = vec![
            Box::new(Label::new("PLAN MANEUVER:").font(font_big, app)),
            Box::new(HRule::new(STYLE.border, 1.0, WIDTH)),
        ];

        let craft = self.craft.unwrap();
        let parent = world
            .get::<&Parent>(craft)
            .expect("should have a parent")
            .id;
        let opts = ManeuverOptions {
            is_landed: world.get::<&Landed>(craft).is_ok(),
            is_orbiting: world.get::<&State>(craft).is_ok(),
            parent_is_solid: !world.get::<&Body>(parent).unwrap().gaseous(),
            can_escape: world.get::<&Parent>(parent).is_ok(),
            bodies: self.get_body_destinations(craft, world),
            crafts: self.get_craft_destinations(craft, world),
            co_located: self.get_craft_colocated(craft, current_et, world),
        };

        if self.selected_kind.is_some_and(|k| !k.available(&opts)) {
            self.selected_kind = None;
        }

        if let Some(dest) = self.selected_destination {
            let still_valid = self
                .selected_kind
                .and_then(|k| k.target_kind())
                .is_some_and(|tk| match tk {
                    TargetKind::Body => opts.bodies.iter().any(|(e, _)| *e == dest),
                    TargetKind::Craft => opts.crafts.iter().any(|(e, _)| *e == dest),
                    TargetKind::CoLocatedCraft => opts.co_located.iter().any(|(e, _)| *e == dest),
                });
            if !still_valid {
                self.selected_destination = None;
                self.computed_plan = None;
            }
        }

        let kind_options: Vec<&ManeuverKind> = ManeuverKind::all()
            .iter()
            .filter(|k| k.available(&opts))
            .collect();
        let selected_kind_idx = self
            .selected_kind
            .as_ref()
            .and_then(|sk| kind_options.iter().position(|k| *k == sk));

        sections.push(Box::new(
            container!(
                Label::new("Maneuver kind:").font(font, app),
                Dropdown::new(
                    "",
                    vec2(WIDTH * 0.5, 40.0),
                    kind_options
                        .iter()
                        .map(|k| (k.label(), ManeuverMessages::SelectKind(*(*k))))
                        .collect(),
                )
                .selected(selected_kind_idx)
                .use_style(&STYLE),
            )
            .flow(Flow::Horizontal)
            .cross_align(Align::Center)
            .padding(vec2(12.0, 0.0)),
        ));

        if let Some(kind) = &self.selected_kind {
            if let Some(target_kind) = kind.target_kind() {
                let destinations = match target_kind {
                    TargetKind::Body => opts.bodies,
                    TargetKind::Craft => opts.crafts,
                    TargetKind::CoLocatedCraft => opts.co_located,
                };
                let selected_dest_idx = self
                    .selected_destination
                    .and_then(|sd| destinations.iter().position(|(e, _)| *e == sd));
                sections.push(Box::new(
                    container!(
                        Label::new("Destination:").font(font, app),
                        Dropdown::new(
                            "",
                            vec2(WIDTH * 0.5, 40.0),
                            destinations
                                .iter()
                                .map(|(entity, name)| {
                                    (name.clone(), ManeuverMessages::SelectDestination(*entity))
                                })
                                .collect(),
                        )
                        .selected(selected_dest_idx)
                        .use_style(&STYLE),
                    )
                    .flow(Flow::Horizontal)
                    .cross_align(Align::Center)
                    .padding(vec2(12.0, 0.0)),
                ));
                sections.push(Box::new(PorkchopPicker::new(
                    vec2(WIDTH, WIDTH * 3.0 / 4.0),
                    self.porkchop_texture_id,
                    DEPART_STEPS,
                    TOF_STEPS,
                    self.selected_cell.clone(),
                    self.optimum.clone(),
                    self.axes.clone(),
                )));
                sections.push(Box::new(
                    Container::new(vec![
                        Box::new(
                            Button::text(vec2(30.0, 30.0), "<")
                                .use_style(&STYLE)
                                .on_click(ManeuverMessages::ShiftPorkchopLeft),
                        ),
                        Box::new(
                            Button::text(vec2(30.0, 30.0), ">")
                                .use_style(&STYLE)
                                .on_click(ManeuverMessages::ShiftPorkchopRight),
                        ),
                        Box::new(
                            Button::text(vec2(30.0, 30.0), "+")
                                .use_style(&STYLE)
                                .on_click(ManeuverMessages::ZoomPorkchopIn),
                        ),
                        Box::new(
                            Button::text(vec2(30.0, 30.0), "-")
                                .use_style(&STYLE)
                                .on_click(ManeuverMessages::ZoomPorkchopOut),
                        ),
                    ])
                    .flow(Flow::Horizontal),
                ));

                if matches!(kind, ManeuverKind::Flyby | ManeuverKind::Transfer) {
                    sections.push(Box::new(
                        Label::bound(self.inclination_text.clone())
                            .font(font, app)
                            .color(STYLE.text),
                    ));
                    sections.push(Box::new(
                        Slider::new(vec2(WIDTH, 16.0), self.theta.clone()).use_style(&STYLE),
                    ));
                }
            }
        }

        sections.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));
        sections.push(Box::new(Label::new("RESULT").font(font_small_bold, app)));
        sections.push(Box::new(
            container![
                Label::new("dv: ").font(font, app).color(STYLE.text),
                Label::bound(self.result_dv_text.clone())
                    .font(font, app)
                    .bind_color(self.dv_color.clone()),
            ]
            .padding(vec2(0.0, 0.0))
            .flow(Flow::Horizontal),
        ));
        sections.push(Box::new(
            container![
                Label::new("Departure: ").font(font, app).color(STYLE.text),
                Label::bound(self.result_depart_text.clone()).font(font, app),
            ]
            .padding(vec2(0.0, 0.0))
            .flow(Flow::Horizontal),
        ));
        sections.push(Box::new(
            container![
                Label::new("Arrival:   ").font(font, app).color(STYLE.text), // pad it out to match departure. This only works because we use a monospaced font
                Label::bound(self.result_date_text.clone()).font(font, app),
            ]
            .padding(vec2(0.0, 0.0))
            .flow(Flow::Horizontal),
        ));

        // Footer buttons
        sections.push(Box::new(HRule::new(STYLE.border, 1.0, WIDTH)));

        sections.push(Box::new(
            container![
                Button::text(vec2(100.0, 30.0), "Close")
                    .use_style(&STYLE)
                    .on_click(ManeuverMessages::Close),
                Button::text(vec2(100.0, 30.0), "Confirm")
                    .use_style_accented(&STYLE)
                    .on_click(ManeuverMessages::Confirm)
                    .active(false)
                    .bound_active(self.can_confirm.clone()),
            ]
            .fixed_width(vec2(WIDTH, 0.0))
            .flow(Flow::Horizontal)
            .cross_align(Align::Center),
        ));

        self.modal = Modal::new(Box::new(
            Container::new(sections)
                .flow(Flow::Vertical)
                .padding(vec2(12.0, 12.0))
                .background_color(STYLE.surface)
                .border(STYLE.border, 1.0),
        ))
        .shown(true);
        self.modal.reposition(app);
    }

    fn get_body_destinations(&self, craft: Entity, world: &World) -> Vec<(Entity, String)> {
        let parent = world
            .get::<&Parent>(craft)
            .expect("craft should have parent")
            .id;
        let mut binding = world.query::<(&State, &Body, &SceneObject, &Parent)>();
        binding
            .iter()
            .filter(|(_, (_, _, _, p))| p.id == parent)
            .map(|(entity, (_state, _body, scene_obj, _parent))| (entity, scene_obj.name.clone()))
            .collect()
    }

    fn get_craft_destinations(&self, craft: Entity, world: &World) -> Vec<(Entity, String)> {
        let parent = world
            .get::<&Parent>(craft)
            .expect("craft should have parent")
            .id;
        // this already excludes landed craft, since they don't have State
        let mut binding = world.query::<(&State, &Craft, &SceneObject, &Parent)>();
        binding
            .iter()
            .filter(|(e, (_, _, _, p))| p.id == parent && craft != *e)
            .map(|(entity, (_state, _body, scene_obj, _parent))| (entity, scene_obj.name.clone()))
            .collect()
    }

    fn get_craft_colocated(
        &self,
        craft: Entity,
        current_et: EphemerisTime,
        world: &World,
    ) -> Vec<(Entity, String)> {
        // Gotta be within 10 km and 1 m/s
        const DOCKING_RANGE: f64 = 10.0 / KM_PER_EARTH_RADIUS;
        const DOCKING_SPEED: f64 = 1.0 / METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR;

        let parent = world
            .get::<&Parent>(craft)
            .expect("craft should have parent")
            .id;
        let mu = world.get::<&Body>(parent).expect("parent must be body").mu;

        let Some(this) = world
            .get::<&State>(craft)
            .ok()
            .and_then(|s| s.propagate(current_et, mu).ok())
        else {
            return vec![];
        };

        // this already excludes landed craft, since they don't have State
        let mut binding = world.query::<(&State, &Craft, &SceneObject, &Parent)>();
        binding
            .iter()
            .filter(|(e, (_, _, _, p))| p.id == parent && craft != *e)
            .filter_map(|(entity, (other_state, _, scene_obj, _))| {
                let other = other_state.propagate(current_et, mu).ok()?;
                let dr = (other.r - this.r).magnitude();
                let dv = (other.v - this.v).magnitude();
                (dr < DOCKING_RANGE && dv < DOCKING_SPEED).then(|| (entity, scene_obj.name.clone()))
            })
            .collect()
    }

    fn compute_porkchop(&self, craft: Entity, world: &World) -> Option<Porkchop> {
        let kind = self.selected_kind.as_ref()?;

        let init_state = world.get::<&State>(craft).ok()?;
        let parent = world
            .get::<&Parent>(craft)
            .expect("craft must have parent")
            .id;
        let parent_body = world.get::<&Body>(parent).expect("parent must be body");

        match kind {
            ManeuverKind::Transfer => {
                let to = self.selected_destination?;
                let target_state = world.get::<&State>(to).unwrap();
                let target_body = world.get::<&Body>(to).unwrap();
                transfer_porkchop(
                    &init_state,
                    &target_state,
                    target_body.body_radius,
                    self.window.as_ref().unwrap(),
                    parent_body.mass(),
                    target_body.mass(),
                    self.theta.get() as f64 * PI * 2.0,
                    DEPART_STEPS,
                    TOF_STEPS,
                )
                .ok()
            }
            ManeuverKind::Flyby => {
                let to = self.selected_destination?;
                let target_state = world.get::<&State>(to).unwrap();
                let target_body = world.get::<&Body>(to).unwrap();
                flyby_porkchop(
                    &init_state,
                    &target_state,
                    target_body.body_radius,
                    self.window.as_ref().unwrap(),
                    parent_body.mass(),
                    target_body.mass(),
                    self.theta.get() as f64 * PI * 2.0,
                    DEPART_STEPS,
                    TOF_STEPS,
                )
                .ok()
            }
            ManeuverKind::Rendezvous => {
                let to = self.selected_destination?;
                let target_state = world.get::<&State>(to).unwrap();
                rendezvous_porkchop(
                    &init_state,
                    &target_state,
                    self.window.as_ref().unwrap(),
                    parent_body.mass(),
                    DEPART_STEPS,
                    TOF_STEPS,
                )
                .ok()
            }
            _ => None,
        }
    }

    fn plan_from_selection(
        &self,
        craft: Entity,
        current_et: EphemerisTime,
        world: &World,
    ) -> Option<ManeuverResult> {
        let kind = self.selected_kind.as_ref()?;

        // Can be Err if craft is landed (no state!)
        let init_state = world.get::<&State>(craft);
        let parent = world
            .get::<&Parent>(craft)
            .expect("craft must have parent")
            .id;
        let parent_body = world.get::<&Body>(parent).expect("parent must be body");

        match kind {
            ManeuverKind::Transfer => {
                let to = self.selected_destination?;
                let target_state = world.get::<&State>(to).unwrap();
                let target_body = world.get::<&Body>(to).unwrap();

                let chop = self.porkchop.as_ref().expect("porkchop was None");

                let (i, j) = self.selected_cell.get();
                let depart_et = chop.depart_at(i);
                let tof = chop.tof_at(j);
                let depart_dv = chop.at(i, j)?.depart_dv;

                match plan_transfer_at(
                    init_state.as_ref().ok()?,
                    &target_state,
                    parent_body.mass(),
                    target_body.mass(),
                    depart_et,
                    tof,
                    depart_dv,
                ) {
                    Ok(plan) => Some(ManeuverResult::Transfer { to, plan }),
                    Err(e) => {
                        println!("plan_transfer_at failed at ({i},{j}) tof={tof:.5}: {e}");
                        None
                    }
                }
            }
            ManeuverKind::Flyby => {
                let to = self.selected_destination?;
                let target_state = world.get::<&State>(to).unwrap();
                let target_body = world.get::<&Body>(to).unwrap();

                let chop = self.porkchop.as_ref()?;

                let (i, j) = self.selected_cell.get();
                let depart_et = chop.depart_at(i);
                let tof = chop.tof_at(j);
                let depart_dv = chop.at(i, j)?.depart_dv;

                let plan = plan_flyby_at(
                    init_state.as_ref().ok()?,
                    &target_state,
                    parent_body.mass(),
                    target_body.mass(),
                    depart_et,
                    tof,
                    depart_dv,
                )
                .ok()?;
                Some(ManeuverResult::Flyby {
                    to,
                    from: parent,
                    plan,
                })
            }
            ManeuverKind::Rendezvous => {
                let with = self.selected_destination?;
                let target_state = world.get::<&State>(with).unwrap();

                let chop = self.porkchop.as_ref()?;

                let (i, j) = self.selected_cell.get();
                let depart_et = chop.depart_at(i);
                let tof = chop.tof_at(j);
                let depart_dv = chop.at(i, j)?.depart_dv;

                let plan = plan_rendezvous_at(
                    init_state.as_ref().ok()?,
                    &target_state,
                    parent_body.mass(),
                    depart_et,
                    tof,
                    depart_dv,
                )
                .ok()?;
                Some(ManeuverResult::Rendezvous { with, plan })
            }
            ManeuverKind::Escape => {
                let parent_state = world.get::<&State>(parent).unwrap();
                let grandparent = world.get::<&Parent>(parent).ok()?; // Could be None if orbiting the Sun
                let grandparent_body = world.get::<&Body>(grandparent.id).unwrap();

                let plan = plan_escape(
                    init_state.as_ref().ok()?,
                    &parent_state,
                    current_et,
                    grandparent_body.mass(),
                    parent_body.mass(),
                );
                Some(ManeuverResult::Escape {
                    to: grandparent.id,
                    from: parent,
                    plan: plan.ok()?,
                })
            }
            ManeuverKind::Land => {
                let target_body = world.get::<&Body>(parent).unwrap();
                let plan = plan_landing(
                    init_state.as_ref().ok()?,
                    target_body.body_radius,
                    current_et,
                    target_body.mu,
                )
                .ok()?;
                Some(ManeuverResult::Land { on: parent, plan })
            }
            ManeuverKind::Launch => {
                let landed = world.get::<&Landed>(craft).ok()?;
                let parent_state = world.get::<&State>(parent).unwrap();
                let grandparent = world.get::<&Parent>(parent).unwrap();
                let grandparent_body = world.get::<&Body>(grandparent.id).unwrap();

                let plan = plan_launch(
                    landed.offset,
                    &parent_state,
                    parent_body.body_radius,
                    current_et,
                    grandparent_body.mass(),
                    parent_body.mass(),
                )
                .ok()?;
                Some(ManeuverResult::Launch { from: parent, plan })
            }
            ManeuverKind::Dock => {
                let with = self.selected_destination?;

                let depart_et = current_et + EphemerisTime::from_secs(5.0);
                let arrive_et = current_et + EphemerisTime::from_mins(30.0);

                Some(ManeuverResult::Dock {
                    with,
                    depart_et,
                    arrive_et,
                })
            }
        }
    }

    fn porkchop_rgba(&self, chop: &Porkchop) -> Vec<u8> {
        let mut bytes = vec![0u8; chop.depart_steps * chop.tof_steps * 4];

        let lo = 0.0;
        let hi = self.budget;
        let span = (hi - lo).max(f64::EPSILON);
        for (n, cell) in chop.cells.iter().enumerate() {
            let pixel: [u8; 4] = match cell {
                Some(c) if c.total <= hi => {
                    let t = ((c.total - lo) / (span)).clamp(0.0, 1.0) as f32;
                    Self::colormap(t)
                }
                _ => [0, 0, 0, 0],
            };
            bytes[n * 4..n * 4 + 4].copy_from_slice(&pixel);
        }

        bytes
    }

    fn colormap(t: f32) -> [u8; 4] {
        let col = oklch(0.45 + 0.42 * t, 0.15, 250.0 + (95.0 - 250.0) * t, 1.0);
        [
            (col.x * 255.0) as u8,
            (col.y * 255.0) as u8,
            (col.z * 255.0) as u8,
            255,
        ]
    }
}
