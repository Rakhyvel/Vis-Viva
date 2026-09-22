use apricot::{
    bvh::BVH,
    high_precision::WorldPosition,
    render_core::{LinePathComponent, ModelComponent, RenderContext},
};
use hecs::{Entity, World};
use nalgebra_glm::{vec3, DVec3};

use crate::{
    astro::{
        epoch::EphemerisTime,
        escape::EscapePlan,
        landing::LandingPlan,
        launch::LaunchPlan,
        rendezvous::RendezvousPlan,
        state::State,
        transfer::{FlybyPlan, TransferPlan},
        units::LITTLE_G,
    },
    components::body::{Parent, SceneObject},
};

pub struct Craft {
    pub part_id: u64,
    pub dry_mass: f64,
    pub engine: Option<Engine>,

    pub command: Option<Command>,
    pub command_scheduled: bool,
    pub line_path_entity: Option<Entity>,
}

#[derive(Clone, Copy)]
pub struct Engine {
    pub fuel_mass: f64, // [kg]
    pub isp: f64,       // [s]
}

pub struct AssociatedEntity {
    pub associate: Entity,
}

#[derive(Clone)]
pub enum Command {
    Launch {
        from: Entity,
        plan: LaunchPlan,
    },
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
    Dock {
        with: Entity,
        arrive_et: EphemerisTime,
    },
}

#[derive(Clone, Copy)]
pub struct ScheduledBurn {
    pub desc: &'static str,
    pub purpose: BurnPurpose,
    pub new_orbit: State,
    pub soi_radius: Option<f64>,
    /// Delta V of the burn, in m/s
    pub dv: f64,
}

impl ScheduledBurn {
    pub fn t(&self) -> EphemerisTime {
        self.new_orbit.t
    }
}

#[derive(Clone, Copy)]
pub enum BurnPurpose {
    Maneuver,
    Landing,
    Launch,
}

impl Command {
    pub fn burn_schedule(&self) -> Vec<ScheduledBurn> {
        match self {
            Command::Transfer { plan, .. } => vec![
                ScheduledBurn {
                    desc: "Departure Burn",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.transfer_state,
                    soi_radius: Some(plan.soi_radius),
                    dv: plan.transfer_dv,
                },
                ScheduledBurn {
                    desc: "Circularization",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.circ_state,
                    soi_radius: Some(plan.soi_radius),
                    dv: plan.circ_dv,
                },
            ],
            Command::Flyby { plan, .. } => vec![ScheduledBurn {
                desc: "Departure Burn",
                purpose: BurnPurpose::Maneuver,
                new_orbit: plan.transfer_state,
                soi_radius: Some(plan.soi_radius),
                dv: plan.transfer_dv,
            }],
            Command::Rendezvous { plan, .. } => vec![
                ScheduledBurn {
                    desc: "Departure Burn",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.transfer_state,
                    soi_radius: None,
                    dv: plan.transfer_dv,
                },
                ScheduledBurn {
                    desc: "Braking Burn",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.rendezvous_state,
                    soi_radius: None,
                    dv: plan.brake_dv,
                },
            ],
            Command::Escape { plan, .. } => {
                vec![ScheduledBurn {
                    desc: "Escape Burn",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.escape_burn,
                    soi_radius: Some(plan.soi_radius),
                    dv: plan.escape_dv,
                }]
            }
            Command::Land { plan, .. } => vec![
                ScheduledBurn {
                    desc: "Deorbit Burn",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.deorbit_burn,
                    soi_radius: None,
                    dv: plan.deorbit_dv,
                },
                ScheduledBurn {
                    desc: "Landing",
                    purpose: BurnPurpose::Landing,
                    new_orbit: plan.landing_burn,
                    soi_radius: None,
                    dv: plan.landing_dv,
                },
            ],
            Command::Launch { plan, .. } => vec![
                ScheduledBurn {
                    desc: "Launch",
                    purpose: BurnPurpose::Launch,
                    new_orbit: plan.launch_burn,
                    soi_radius: None,
                    dv: plan.launch_dv,
                },
                ScheduledBurn {
                    desc: "Circularization",
                    purpose: BurnPurpose::Maneuver,
                    new_orbit: plan.circ_burn,
                    soi_radius: None,
                    dv: plan.circ_dv,
                },
            ],
            Command::Dock { .. } => vec![
                // no burns technically
            ],
        }
    }

    pub fn transition_schedule(&self) -> Vec<(&'static str, EphemerisTime)> {
        match self {
            Command::Transfer { plan, .. } => vec![("Enters SOI", plan.flyby_state.t)],
            Command::Flyby { plan, .. } => vec![
                ("Enters SOI", plan.flyby_state.t),
                ("Leaves SOI", plan.exit_state.t),
            ],
            Command::Escape { plan, .. } => vec![("Leaves SOI", plan.exit_state.t)],
            Command::Land { .. }
            | Command::Launch { .. }
            | Command::Rendezvous { .. }
            | Command::Dock { .. } => vec![],
        }
    }

    pub fn title_parts(&self) -> (&'static str, Entity) {
        match self {
            Command::Transfer { to, .. } => ("Transfer to", *to),
            Command::Flyby { to, .. } => ("Flyby", *to),
            Command::Rendezvous { with, .. } => ("Rendezvous with", *with),
            Command::Escape { from, .. } => ("Escape from", *from),
            Command::Land { on, .. } => ("Land on", *on),
            Command::Launch { from, .. } => ("Launch from", *from),
            Command::Dock { with, .. } => ("Dock with", *with),
        }
    }
}

pub struct Landed {
    pub offset: DVec3,
}

pub fn spawn_craft(
    craft: Craft,
    mut scene_obj: SceneObject,
    parent: Parent,
    world: &mut World,
    renderer: &RenderContext,
    bvh: &mut BVH<Entity>,
) -> Entity {
    let craft_mesh = renderer.get_mesh_id_from_name("cone").unwrap();

    let position: DVec3 = vec3(0., 0., 0.);
    let scale_vec: DVec3 = vec3(0.01, 0.01, 0.01);

    let texture_id = renderer.get_texture_id_from_name("europa").unwrap();

    let craft_entity = world.spawn((
        WorldPosition { pos: position },
        ModelComponent::new(
            craft_mesh,
            texture_id,
            nalgebra_glm::convert(position),
            nalgebra_glm::convert(scale_vec),
        ),
    ));

    let bvh_node_id = bvh.insert(
        craft_entity,
        renderer
            .get_mesh_aabb(craft_mesh)
            .scale(nalgebra_glm::convert(scale_vec))
            .translate(nalgebra_glm::convert(position)),
    );

    scene_obj.bvh_node_id = Some(bvh_node_id);

    world
        .insert(craft_entity, (scene_obj, parent, craft))
        .unwrap();

    craft_entity
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

impl Craft {
    pub fn total_remaining_dv(&self, cargo_kg: f64) -> f64 {
        let Some(e) = self.engine else { return 0.0 };
        let m0 = self.total_mass(cargo_kg);
        e.isp * LITTLE_G * (m0 / (m0 - e.fuel_mass).max(1e-9)).ln()
    }

    /// Returns the total mass of the spacecraft, in kg
    pub fn total_mass(&self, cargo_kg: f64) -> f64 {
        self.dry_mass + self.engine.map_or(0.0, |e| e.fuel_mass) + cargo_kg
    }

    pub fn burn(&mut self, requested_dv: f64, cargo_kg: f64) {
        let m0 = self.total_mass(cargo_kg);
        let Some(e) = &mut self.engine else { return };
        let mf = m0 / (requested_dv / (e.isp * LITTLE_G)).exp();
        e.fuel_mass -= (m0 - mf).min(e.fuel_mass)
    }
}
