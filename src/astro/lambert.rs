use std::f64::consts::PI;

use nalgebra_glm::DVec3;

use crate::astro::stumpff::stumpff_c2_c3;

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
    let mut y = 0.0;
    let mut converged = false;
    for _ in 0..1000 {
        y = r1_mag + r2_mag + a * (phi * c3 - 1.0) / c2.sqrt();

        if y < 0.0 {
            // no transfer has y < 0
            if a > 0.0 {
                phi_lower = phi;
            } else {
                phi_upper = phi;
            }
        } else {
            let cur_tof = ((y / c2).sqrt().powi(3) * c3 + a * y.sqrt()) / mu.sqrt();

            if (cur_tof - tof).abs() < tol {
                converged = true;
                break;
            }

            if cur_tof < tof {
                phi_lower = phi;
            } else {
                phi_upper = phi;
            }
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

#[cfg(test)]
mod tests {
    use nalgebra_glm::DVec3;

    use crate::astro::{
        epoch::EphemerisTime,
        lambert::{lambert, TransferKind},
        state::State,
    };

    #[test]
    fn recovers_the_orbit_between_two_points() {
        const MU: f64 = 3.0;
        let t0 = EphemerisTime::epoch();
        let ellipse = State::from_kepler(2.0, 0.3, 0.4, 0.5, 0.6, 0.4, t0, MU);
        let hyperbola = State::from_kepler(-1.0, 1.8, 0.4, 0.5, 0.6, -0.5, t0, MU);
        // the ellipse's period is ~10.3 years, under half an orbit takes the short way, over half the long way
        let cases = [
            (ellipse, 2.5),
            (ellipse, 7.2),
            (ellipse, 9.7),
            // fast transfers need hyperbolic arcs, including a ~2 degree hop
            (hyperbola, 0.01),
            (hyperbola, 0.3),
            (hyperbola, 3.0),
            (hyperbola, 100.0),
        ];
        for (start, tof) in cases {
            let end = start
                .propagate(t0 + EphemerisTime::from_years(tof), MU)
                .unwrap();
            let short = start.r.cross(&end.r).dot(&start.angular_momentum()) > 0.0;
            let kind = if short {
                TransferKind::Short
            } else {
                TransferKind::Long
            };

            let (v1, v2) = lambert(start.r, end.r, tof, MU, kind)
                .unwrap_or_else(|| panic!("no transfer found for tof {tof}"));

            let err1 = (v1 - start.v).norm() / start.v.norm();
            let err2 = (v2 - end.v).norm() / end.v.norm();
            assert!(
                err1 < 1e-6 && err2 < 1e-6,
                "tof {tof}: velocity errors {err1:e}, {err2:e}"
            );
        }
    }

    #[test]
    fn opposite_points_have_no_unique_transfer() {
        // 180 degrees apart, every plane through both points works equally well
        let r1 = DVec3::new(1.5, 0.2, 0.1);
        let r2 = -2.0 * r1;
        assert!(lambert(r1, r2, 1.0, 3.0, TransferKind::Short).is_none());
        assert!(lambert(r1, r2, 1.0, 3.0, TransferKind::Long).is_none());
    }
}
