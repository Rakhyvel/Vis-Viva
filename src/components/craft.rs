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
    components::body::{Body, Parent, SceneObject},
};

pub struct Craft {
    pub payload: Payload,
    // FILO, last one is the one that's burning
    // <====<---
    pub stages_stack: Vec<Stage>,

    pub command: Option<Command>,
    pub command_scheduled: bool,
    pub line_path_entity: Option<Entity>,
}

#[derive(Clone)]
pub struct Payload {
    #[allow(dead_code)]
    pub name: String,
    pub dry_mass: f64,
    // TODO: add things like component slots and components
}

#[derive(Clone)]
pub struct Stage {
    #[allow(unused)]
    pub name: String,

    pub dry_mass: f64,  // [kg]
    pub fuel_mass: f64, // [kg]
    #[allow(unused)]
    pub max_fuel_mass: f64, // [kg]

    pub thrust_kn: f64, // [kN]
    pub isp: f64,       // [s]
}

pub struct AssociatedEntity {
    pub associate: Entity,
}

#[derive(Clone)]
pub enum Command {
    Launch { plan: LaunchPlan },
    Transfer { to: Entity, plan: TransferPlan },
    Flyby { to: Entity, plan: FlybyPlan },
    Rendezvous { plan: RendezvousPlan },
    Escape { to: Entity, plan: EscapePlan },
    Land { plan: LandingPlan },
}

#[derive(Clone, Copy)]
pub enum BurnPurpose {
    Maneuver,
    Landing,
    Launch,
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Command::Transfer { .. } => "Transfer",
            Command::Flyby { .. } => "Flyby",
            Command::Rendezvous { .. } => "Rendezvous",
            Command::Escape { .. } => "Escape",
            Command::Land { .. } => "Land",
            Command::Launch { .. } => "Launch",
        }
    }

    pub fn burn_schedule(&self) -> Vec<(&'static str, BurnPurpose, EphemerisTime)> {
        match self {
            Command::Transfer { plan, .. } => vec![
                (
                    "Departure burn",
                    BurnPurpose::Maneuver,
                    plan.transfer_state.t,
                ),
                ("Circularization", BurnPurpose::Maneuver, plan.circ_state.t),
            ],
            Command::Flyby { plan, .. } => vec![(
                "Departure burn",
                BurnPurpose::Maneuver,
                plan.transfer_state.t,
            )],
            Command::Rendezvous { plan, .. } => vec![
                (
                    "Departure burn",
                    BurnPurpose::Maneuver,
                    plan.transfer_state.t,
                ),
                (
                    "Braking burn",
                    BurnPurpose::Maneuver,
                    plan.rendezvous_state.t,
                ),
            ],
            Command::Escape { plan, .. } => {
                vec![("Escape burn", BurnPurpose::Maneuver, plan.escape_burn.t)]
            }
            Command::Land { plan } => vec![
                ("Deorbit burn", BurnPurpose::Maneuver, plan.deorbit_burn.t),
                ("Landing", BurnPurpose::Landing, plan.landing_burn.t),
            ],
            Command::Launch { plan } => vec![
                ("Launch", BurnPurpose::Launch, plan.launch_burn.t),
                ("Circularization", BurnPurpose::Maneuver, plan.circ_burn.t),
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
            Command::Land { .. } | Command::Launch { .. } | Command::Rendezvous { .. } => vec![],
        }
    }
}

pub struct Landed {
    pub offset: DVec3,
}

pub fn spawn_landed_craft(
    payload: Payload,
    stages_stack: Vec<Stage>,
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

    let parent_radius = { world.get::<&Body>(parent.id).unwrap().body_radius };

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
        .insert(
            craft_entity,
            (
                scene_obj,
                parent,
                Landed {
                    offset: vec3(0.0, parent_radius, 0.0),
                },
                Craft {
                    stages_stack,
                    payload,
                    command: None,
                    command_scheduled: false,
                    line_path_entity: None,
                },
            ),
        )
        .unwrap();

    craft_entity
}

pub fn spawn_orbiting_craft(
    payload: Payload,
    stages_stack: Vec<Stage>,
    mut scene_obj: SceneObject,
    parent: Parent,
    state: State,
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
        .insert(
            craft_entity,
            (
                scene_obj,
                parent,
                state,
                Craft {
                    stages_stack,
                    payload,
                    command: None,
                    command_scheduled: false,
                    line_path_entity: None,
                },
            ),
        )
        .unwrap();

    let parent_mu = world.get::<&Body>(parent.id).unwrap().mu;
    let parent_world_pos = world.get::<&WorldPosition>(parent.id).unwrap().pos;

    replace_line_path(
        world,
        renderer,
        craft_entity,
        Some((
            WorldPosition {
                pos: parent_world_pos,
            },
            parent,
            LinePathComponent::new(
                state
                    .generate_orbit_vertices(8192, parent_mu, None)
                    .unwrap(),
            ),
            AssociatedEntity {
                associate: craft_entity,
            },
        )),
    );

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
    #[allow(dead_code)]
    pub fn current_stage_dv(&self) -> f64 {
        let stage = self.stages_stack.last();
        if stage.is_none() {
            return 0.0;
        }
        let stage = stage.unwrap();

        let m0 = self.total_mass();
        let mf = m0 - stage.fuel_mass;

        stage.isp * LITTLE_G * (m0 / mf).ln()
    }

    pub fn total_remaining_dv(&self) -> f64 {
        let mut total_mass = self.total_mass();
        let mut total_dv = 0.0;

        // iterate stages from last (burning) to first (payload)
        for stage in self.stages_stack.iter().rev() {
            let m0 = total_mass;
            let mf = total_mass - stage.fuel_mass;
            total_dv += stage.isp * LITTLE_G * (m0 / mf.max(1e-9)).ln();
            total_mass -= stage.fuel_mass + stage.dry_mass; // jettison stage
        }

        total_dv
    }

    /// Returns the total mass of the spacecraft, in kg
    pub fn total_mass(&self) -> f64 {
        let payload_mass = self.payload.dry_mass;

        let stage_mass: f64 = self
            .stages_stack
            .iter()
            .map(|s| s.dry_mass + s.fuel_mass)
            .sum();

        payload_mass + stage_mass
    }

    pub fn twr(&self) -> Option<f64> {
        let total_mass_kg = self.total_mass();
        let bottom_stage = self.stages_stack.last()?;
        let thrust_n = bottom_stage.thrust_kn * 1000.0;
        Some(thrust_n / (total_mass_kg * 9.81))
    }

    pub fn burn(&mut self, mut requested_dv: f64) {
        while requested_dv > 0.0 {
            let m0 = self.total_mass();

            let stage = match self.stages_stack.last_mut() {
                Some(s) => s,
                None => break,
            };

            // max dv this stage can provide RIGHT NOW
            let stage_fuel = stage.fuel_mass;

            let mf = m0 - stage_fuel;
            let max_dv = stage.isp * LITTLE_G * (m0 / mf.max(1e-9)).ln();

            if max_dv >= requested_dv {
                // stage can handle it fully
                let mf_needed = m0 / (requested_dv / (stage.isp * LITTLE_G)).exp();
                let fuel_used = (m0 - mf_needed).min(stage.fuel_mass);

                stage.fuel_mass -= fuel_used;
                return;
            } else {
                // burn entire stage
                let mf = m0 - stage.fuel_mass;
                debug_assert!(stage.fuel_mass <= m0, "fuel mass exceeds total mass");
                let dv_used = stage.isp * LITTLE_G * (m0 / mf.max(1e-9)).ln();

                requested_dv -= dv_used;

                stage.fuel_mass = 0.0;
                self.stages_stack.pop(); // discard stage
            }
        }
    }
}
