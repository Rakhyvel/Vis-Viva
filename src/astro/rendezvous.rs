use nalgebra_glm::DVec3;

use crate::astro::{
    epoch::EphemerisTime,
    lambert::lambert,
    porkchop::{best_branch, SweepWindow},
    porkchop::{Cell, Porkchop},
    state::State,
    units::{G, METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR},
};

#[derive(Clone, Copy)]
pub struct RendezvousPlan {
    pub transfer_state: State,
    pub rendezvous_state: State,
    pub transfer_dv: f64,
    pub brake_dv: f64,
    // TODO: Phasing maneuver, with player input for the number of revs to wait
}

pub fn rendezvous_porkchop(
    craft_state: &State,
    target_state: &State,
    w: &SweepWindow,
    parent_mass: f64, // in earth masses
    depart_steps: usize,
    tof_steps: usize,
) -> Result<Porkchop, String> {
    let mu = G * parent_mass;

    let chop = Porkchop::compute(
        w.start,
        w.sweep,
        w.tof_min,
        w.tof_max,
        depart_steps,
        tof_steps,
        |et, tof| {
            let craft = craft_state.propagate(et, mu).ok()?;
            let target = target_state
                .propagate(et + EphemerisTime::from_years(tof), mu)
                .ok()?;

            best_branch(|k| {
                let (v1, v2) = lambert(craft.r, target.r, tof, mu, k)?;
                let depart_dv = v1 - craft.v;
                let brake = (target.v - v2).norm();
                Some(Cell {
                    total: depart_dv.norm() + brake,
                    depart_dv,
                    arrival_dv: brake,
                })
            })
        },
    );

    Ok(chop)
}

pub fn plan_rendezvous_at(
    craft_state: &State,
    target_state: &State,
    parent_mass: f64, // in earth masses
    depart_et: EphemerisTime,
    tof: f64,
    depart_dv: DVec3,
) -> Result<RendezvousPlan, String> {
    let mu = G * parent_mass;

    let mut transfer_state = craft_state.propagate(depart_et, mu)?;
    transfer_state.v += depart_dv;

    let arrival_et = depart_et + EphemerisTime::from_years(tof);
    let arrive = transfer_state.propagate(arrival_et, mu)?;
    let tgt = target_state.propagate(arrival_et, mu)?;
    let brake_dv = (tgt.v - arrive.v).norm();

    Ok(RendezvousPlan {
        transfer_state,
        transfer_dv: depart_dv.norm() * METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR,
        rendezvous_state: tgt,
        brake_dv: brake_dv * METERS_PER_SECOND_PER_EARTH_RADII_PER_YEAR,
    })
}
