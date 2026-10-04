use std::f64::consts::PI;

use nalgebra_glm::DVec3;

use crate::astro::stumpff::stumpff_c2_c3;
use crate::astro::{epoch::EphemerisTime, state::State};

const LAMBERT_EPSILON: f64 = 1e-4; // General epsilon

#[derive(Clone, Copy)]
pub enum TransferKind {
    Short,
    Long,
}

/// Solve using Vallado's algorithm
pub fn lambert(
    r1: DVec3,
    r2: DVec3,
    tof: f64,
    mu: f64,
    kind: TransferKind,
) -> Option<(DVec3, DVec3)> {
    assert!(tof > 0.0, "tof was non-positive");
    assert!(mu > 0.0, "grav param was non-positive");

    let dm = match kind {
        TransferKind::Short => 1.0,
        TransferKind::Long => -1.0,
    };

    let r1_mag = r1.norm();
    let r2_mag = r2.norm();

    let cos_dnu = r1.dot(&r2) / (r1_mag * r2_mag);
    let a = dm * (r1_mag * r2_mag * (1.0 + cos_dnu)).sqrt();
    if a.abs() < LAMBERT_EPSILON {
        return None;
    }

    let tol = (tof * 1e-9).max(1e-12);

    let mut phi_upper = 4.0 * PI * PI;
    let mut phi_lower = -4.0 * PI * PI;
    let mut phi = 0.0;
    let mut c2 = 0.5_f64;
    let mut c3 = 1.0_f64 / 6.0;
    let mut cur_tof;
    let mut y = 0.0;
    let mut converged = false;
    for _ in 0..1000 {
        y = r1_mag + r2_mag + a * (phi * c3 - 1.0) / c2.sqrt();

        if y < 0.0 {
            // short way need larger phi, long way needs smaller
            let dir = -a.signum() * 0.1;
            for _ in 0..500 {
                phi += dir;
                let (nc2, nc3) = stumpff_c2_c3(phi);
                y = r1_mag + r2_mag + a * (phi * nc3 - 1.0) / nc2.sqrt();
                if y >= 0.0 {
                    c2 = nc2;
                    c3 = nc3;
                    break;
                }
            }
            if y < 0.0 {
                return None;
            }
            // keep the bisection out of the region we just escaped
            if a > 0.0 {
                phi_lower = phi;
            } else {
                phi_upper = phi;
            }
        }

        cur_tof = ((y / c2).sqrt().powi(3) * c3 + a * y.sqrt()) / mu.sqrt();

        if (cur_tof - tof).abs() < tol {
            converged = true;
            break;
        }

        if cur_tof < tof {
            phi_lower = phi;
        } else {
            phi_upper = phi;
        }
        phi = (phi_upper + phi_lower) / 2.0;

        (c2, c3) = stumpff_c2_c3(phi);
    }

    if !converged {
        return None;
    }

    let f = 1.0 - y / r1_mag;
    let g = a * (y / mu).sqrt();
    let gdot = 1.0 - y / r2_mag;

    let v1 = (r2 - f * r1) / g;
    let v2 = (gdot * r2 - r1) / g;

    Some((v1, v2))
}

#[test]
fn test_lambert_recovers_velocity() {
    let mu = 1.0;
    let r = 2.0;
    let init_state = State {
        r: DVec3::new(r, 0.0, 0.0),
        v: DVec3::new(0.0, (mu / r).sqrt(), 0.0),
        t: EphemerisTime::new(0),
    };

    // Use a quarter period so r1 and r2 are 90 degrees apart
    let period = 2.0 * PI * (r.powi(3) / mu).sqrt();
    let departure_et = EphemerisTime::new(0);
    let arrival_et = EphemerisTime::from_years(period / 4.0);
    let tof = (arrival_et - departure_et).as_years();

    let depart_state = init_state.propagate(departure_et, mu).unwrap();
    let arrival_state = init_state.propagate(arrival_et, mu).unwrap();

    let Some((v1, _)) = lambert(
        depart_state.r,
        arrival_state.r,
        tof,
        mu,
        TransferKind::Short,
    ) else {
        panic!("lambert didn't find a solution")
    };

    let err = (v1 - depart_state.v).norm();
    println!("lambert v: {:?}", v1);
    println!("true    v: {:?}", depart_state.v);
    println!("error:     {err:.2e}");
    assert!(err < 1e-6, "lambert velocity error too large: {err}");
}
