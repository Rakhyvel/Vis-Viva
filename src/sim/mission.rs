use hecs::Entity;

use crate::astro::{
    epoch::EphemerisTime,
    escape::EscapePlan,
    landing::LandingPlan,
    launch::LaunchPlan,
    rendezvous::RendezvousPlan,
    state::State,
    transfer::{FlybyPlan, TransferPlan},
};

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
        depart_et: EphemerisTime,
    },
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

    pub fn total_dv(&self) -> f64 {
        match self {
            Command::Transfer { plan, .. } => plan.transfer_dv + plan.circ_dv,
            Command::Flyby { plan, .. } => plan.transfer_dv,
            Command::Rendezvous { plan, .. } => plan.transfer_dv + plan.brake_dv,
            Command::Escape { plan, .. } => plan.escape_dv,
            Command::Land { plan, .. } => plan.deorbit_dv + plan.landing_dv,
            Command::Launch { plan, .. } => plan.launch_dv + plan.circ_dv,
            Command::Dock { .. } => 0.0,
        }
    }

    pub fn departure_et(&self) -> EphemerisTime {
        match self {
            Command::Transfer { plan, .. } => plan.transfer_state.t,
            Command::Flyby { plan, .. } => plan.transfer_state.t,
            Command::Rendezvous { plan, .. } => plan.transfer_state.t,
            Command::Escape { plan, .. } => plan.escape_burn.t,
            Command::Land { plan, .. } => plan.deorbit_burn.t,
            Command::Launch { plan, .. } => plan.launch_burn.t,
            Command::Dock { depart_et, .. } => *depart_et,
        }
    }

    pub fn arrival_et(&self) -> EphemerisTime {
        match self {
            Command::Transfer { plan, .. } => plan.circ_state.t,
            Command::Flyby { plan, .. } => plan.flyby_state.t,
            Command::Rendezvous { plan, .. } => plan.rendezvous_state.t,
            Command::Escape { plan, .. } => plan.exit_state.t,
            Command::Land { plan, .. } => plan.landing_burn.t,
            Command::Launch { plan, .. } => plan.circ_burn.t,
            Command::Dock { arrive_et, .. } => *arrive_et,
        }
    }

    pub fn can_afford(&self, craft_dv: f64) -> bool {
        self.total_dv() <= craft_dv
    }
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
