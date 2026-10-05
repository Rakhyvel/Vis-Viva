use nalgebra_glm::DVec3;

use crate::astro::{epoch::EphemerisTime, state::State};

pub fn sphere_of_influence(orbital_radius: f64, body_mass: f64, parent_mass: f64) -> f64 {
    orbital_radius * (body_mass / parent_mass).powf(2.0 / 5.0)
}

pub fn find_apoapsis(orbit: &State, current_et: EphemerisTime, mu: f64) -> Result<State, String> {
    let rdotv_at_t = |et: EphemerisTime| -> Result<f64, String> {
        let state = orbit.propagate(et, mu)?;
        Ok(state.r.dot(&state.v))
    };

    let period = orbit
        .period(mu)
        .ok_or("only periodic orbits have apoapsides")?;

    let dt = EphemerisTime::from_years(period / 100.0); // 100 steps per orbit

    const TOL: f64 = 1e-6;
    if orbit.ecc(mu) < TOL {
        // orbit is circular, apoapsis isn't defined. Just pick this point
        return orbit.propagate(current_et + dt, mu);
    }

    let mut lo = current_et;
    let mut hi = current_et;
    let max_et = current_et + EphemerisTime::from_years(period);

    // If already past apoapsis (rdotv < 0), march lo forward until rdotv > 0
    // so lo is guaranteed to be on the positive side
    while rdotv_at_t(lo)? < 0.0 {
        lo += dt;
        hi = lo;
        if lo > max_et {
            return Err(String::from("apoapsis not found within one period"));
        }
    }

    // now march hi forward until rdotv goes negative
    while rdotv_at_t(hi)? > 0.0 {
        hi += dt;
        if hi > max_et {
            return Err(String::from("apoapsis not found within one period"));
        }
    }

    // Binary search for rdotv = 0 crossing (positive -> negative)
    const ITERATIONS: usize = 50;
    for _ in 0..ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        if rdotv_at_t(mid)? > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    orbit.propagate(hi, mu)
}

pub fn find_periapsis(orbit: &State, current_et: EphemerisTime, mu: f64) -> Result<State, String> {
    let rdotv_at_t = |et: EphemerisTime| -> Result<f64, String> {
        let state = orbit.propagate(et, mu)?;
        Ok(state.r.dot(&state.v))
    };

    const TOL: f64 = 1e-6;
    let ecc = orbit.ecc(mu);
    if ecc < TOL {
        // orbit is circular, periapsis isn't defined. Just pick this point
        return orbit.propagate(current_et + EphemerisTime::from_secs(60.0), mu);
    }

    if ecc >= 1.0 && rdotv_at_t(current_et)? > 0.0 {
        // hyperbolic orbit and we're already past the periapsis
        return orbit.propagate(current_et + EphemerisTime::from_secs(60.0), mu);
    }

    // Use period-based step for elliptical, or r/v based step for hyperbolic
    let dt = if let Some(period) = orbit.period(mu) {
        EphemerisTime::from_years(period / 100.0) // 100 steps per orbit
    } else {
        // Hyperbolic - use time to travel one radius at current speed
        let x = orbit.propagate(current_et, mu)?;
        let r = x.r.norm();
        let v = x.v.norm();
        EphemerisTime::from_years(r / v / 10.0)
    }
    .min(EphemerisTime::from_years(100.0));

    let mut lo = current_et;
    let mut hi = current_et;
    let max_et = current_et + EphemerisTime::from_years(orbit.period(mu).unwrap_or(10.0));

    // If already past periapsis (rdotv > 0), march lo forward until rdotv < 0
    while rdotv_at_t(lo)? > 0.0 {
        lo += dt;
        hi = lo;
        if lo > max_et {
            return Err(String::from("periapsis not found within one period"));
        }
    }

    // Now march hi forward until rdotv goes positive
    while rdotv_at_t(hi)? < 0.0 {
        hi += dt;
        if hi > max_et {
            return Err(String::from("periapsis not found within one period"));
        }
    }

    // Binary search for the zero crossing
    const ITERATIONS: usize = 50;
    for _ in 0..ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        if rdotv_at_t(mid)? < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    orbit.propagate(hi, mu)
}

pub fn find_soi_entry(
    transfer_orbit: &State,
    target_orbit: &State,
    target_soi: f64,
    tof: f64,
    mu: f64,
) -> Result<EphemerisTime, String> {
    let distance_at_t = |t: f64| -> Result<f64, String> {
        let sample_time = transfer_orbit.t + EphemerisTime::from_years(t * tof);
        let craft_pos = transfer_orbit.propagate(sample_time, mu)?.r;
        let target_pos = target_orbit.propagate(sample_time, mu)?.r;
        Ok((craft_pos - target_pos).norm())
    };

    // Binary search between 0 and 1 (normalized departure and periapsis)
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;

    const ITERATIONS: usize = 50;
    for _ in 0..ITERATIONS {
        let mid = (lo + hi) / 2.0;
        if distance_at_t(mid)? < target_soi {
            hi = mid; // inside SOI, search earlier
        } else {
            lo = mid; // outside SOI, search later
        }
    }

    Ok(transfer_orbit.t + EphemerisTime::from_years(hi * tof))
}

pub fn find_soi_exit(orbit: &State, soi: f64, mu: f64) -> Result<EphemerisTime, String> {
    // First find a rough bracket by marching forward
    let dt_coarse = EphemerisTime::from_years(1.0 / 365.0); // 1 day steps
    let mut t = orbit.t + EphemerisTime::from_secs(60.0);

    // March until we're outside the SOI
    loop {
        t += dt_coarse;
        let pos = orbit.propagate(t, mu)?.r;
        if pos.norm() >= soi {
            break;
        }
        // Safety limit - 10 years
        if t > orbit.t + EphemerisTime::from_years(10.0) {
            return Err(String::from("SOI exit not found within 10 years"));
        }
    }

    // Binary search to refine
    let mut lo = t - dt_coarse;
    let mut hi = t;

    const ITERATIONS: usize = 50;
    for _ in 0..ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        let pos = orbit.propagate(mid, mu)?.r;
        if pos.norm() < soi {
            lo = mid; // inside SOI, search later
        } else {
            hi = mid; // outisde SOI, search earlier
        }
    }

    Ok(hi)
}

pub fn circularization(orbit: &State, mu: f64) -> (State, f64) {
    let r = orbit.r;
    let v = orbit.v;

    let r_mag = r.norm();

    // the velocity if we were in a circular orbit
    let v_circ_mag = (mu / r_mag).sqrt();

    // convert the scalar velocity to a scalar
    let r_hat = r.normalize();
    let h_hat = r.cross(&v).normalize();
    let t_hat = h_hat.cross(&r_hat);

    let v_circ = t_hat * v_circ_mag;

    // circ dv is just the differnce between v_circ and v

    (
        State {
            r: orbit.r,
            v: v_circ,
            t: orbit.t,
        },
        (v_circ - v).norm(),
    )
}

pub fn capture_at_periapsis_dv(r_rel: DVec3, v_rel: DVec3, mu: f64) -> f64 {
    let h = r_rel.cross(&v_rel).norm();
    let energy = v_rel.norm_squared() / 2.0 - mu / r_rel.norm();
    let e = (1.0 + 2.0 * energy * h * h / (mu * mu)).max(0.0).sqrt();
    let r_p = h * h / mu / (1.0 + e);
    let v_p = h / r_p;
    v_p - (mu / r_p).sqrt()
}

pub fn impact_parameter(r_p: f64, v_inf: f64, mu: f64) -> f64 {
    r_p * (1.0 + 2.0 * mu / (r_p * v_inf * v_inf)).sqrt()
}

pub fn get_grandparent_state(
    craft_state: &State,
    parent_state: &State,
    soi: f64,
    grandparent_mu: f64,
    parent_mu: f64,
) -> Result<State, String> {
    let soi_exit_et = find_soi_exit(craft_state, soi, parent_mu)?;

    // Craft state at SOI exit in parent-relative frame
    let craft_at_exit = craft_state.propagate(soi_exit_et, parent_mu)?;

    // Parent state at SOI exit in grandparent frame
    let parent_at_exit = parent_state.propagate(soi_exit_et, grandparent_mu)?;

    // Recontextualize to grandparent frame
    Ok(State {
        r: craft_at_exit.r + parent_at_exit.r,
        v: craft_at_exit.v + parent_at_exit.v,
        t: soi_exit_et,
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::astro::units::{EARTH_MASSES_PER_SUN_MASS, EARTH_RADII_PER_AU, G, SUN_MU};

    #[test]
    fn earths_soi_is_about_925_000_km() {
        // the textbook value, from Earth's 1 AU orbit and the Earth/Sun mass ratio
        let soi_km = sphere_of_influence(1.496e8, 1.0, EARTH_MASSES_PER_SUN_MASS);
        assert!((soi_km / 9.25e5 - 1.0).abs() < 0.01, "{soi_km} km");
    }

    #[test]
    fn apoapsis_is_found_within_the_next_orbit() {
        const MU: f64 = 10.0;
        let (a, e) = (10.0, 0.3);
        let t0 = EphemerisTime::epoch();
        // before apoapsis, and past it so the search has to go round through periapsis
        for nu in [0.4, 4.0] {
            let orbit = State::from_kepler(a, e, 0.1, 0.2, 0.3, nu, t0, MU);
            let period = 2.0 * PI * (a.powi(3) / MU).sqrt();

            let apo = find_apoapsis(&orbit, t0, MU).unwrap();

            assert!(
                (apo.r.norm() / (a * (1.0 + e)) - 1.0).abs() < 1e-9,
                "nu {nu}: r = {}",
                apo.r.norm()
            );
            assert!(
                apo.t > t0 && apo.t < t0 + EphemerisTime::from_years(period),
                "nu {nu}"
            );
        }
    }

    #[test]
    fn periapsis_is_found_within_the_next_orbit() {
        const MU: f64 = 10.0;
        let (a, e) = (10.0, 0.3);
        let t0 = EphemerisTime::epoch();
        // past periapsis so the search goes round through apoapsis, and before it
        for nu in [2.0, 4.0] {
            let orbit = State::from_kepler(a, e, 0.1, 0.2, 0.3, nu, t0, MU);
            let period = 2.0 * PI * (a.powi(3) / MU).sqrt();

            let peri = find_periapsis(&orbit, t0, MU).unwrap();

            assert!(
                (peri.r.norm() / (a * (1.0 - e)) - 1.0).abs() < 1e-9,
                "nu {nu}: r = {}",
                peri.r.norm()
            );
            assert!(
                peri.t > t0 && peri.t < t0 + EphemerisTime::from_years(period),
                "nu {nu}"
            );
        }
    }

    #[test]
    fn hyperbolic_periapsis_is_found_on_the_way_in() {
        let mu = G; // Earth
                    // periapsis 2 ER, inbound
        let (a, e) = (-4.0, 1.5);
        let t0 = EphemerisTime::epoch();
        let orbit = State::from_kepler(a, e, 0.1, 0.2, 0.3, -1.0, t0, mu);

        let peri = find_periapsis(&orbit, t0, mu).unwrap();

        assert!(
            (peri.r.norm() / (a * (1.0 - e)) - 1.0).abs() < 1e-9,
            "r = {}",
            peri.r.norm()
        );
        assert!(peri.t > t0);
    }

    #[test]
    fn soi_entry_is_where_the_craft_first_reaches_the_soi() {
        const MU: f64 = 10.0;
        const SOI: f64 = 0.5;
        let t0 = EphemerisTime::epoch();
        let tof = 3.0;
        let arrival = t0 + EphemerisTime::from_years(tof);

        // the target on a circular orbit, and a craft that reaches the same point at `arrival`,
        // moving 0.5 faster outwards, built backwards from that meeting
        let target = State::circular(10.0, t0, MU);
        let target_then = target.propagate(arrival, MU).unwrap();
        let craft_then = State {
            r: target_then.r,
            v: target_then.v + 0.5 * target_then.r.normalize(),
            t: arrival,
        };
        let craft = craft_then.propagate(t0, MU).unwrap();

        let entry = find_soi_entry(&craft, &target, SOI, tof, MU).unwrap();

        let gap = |t: EphemerisTime| {
            (craft.propagate(t, MU).unwrap().r - target.propagate(t, MU).unwrap().r).norm()
        };
        assert!(
            (gap(entry) / SOI - 1.0).abs() < 1e-6,
            "gap at entry {}",
            gap(entry)
        );
        assert!(entry > t0 && entry < arrival);
    }

    #[test]
    fn soi_exit_is_where_the_craft_leaves_the_soi() {
        let mu = G; // Earth
        const SOI: f64 = 145.0; // ~Earth's SOI, in ER
        let t0 = EphemerisTime::epoch();
        // escaping from a 2 ER periapsis
        let orbit = State::from_kepler(-4.0, 1.5, 0.1, 0.2, 0.3, 0.0, t0, mu);

        let exit = find_soi_exit(&orbit, SOI, mu).unwrap();

        let r = orbit.propagate(exit, mu).unwrap().r.norm();
        assert!((r / SOI - 1.0).abs() < 1e-9, "r at exit {r}");
        let just_before = orbit
            .propagate(exit - EphemerisTime::from_secs(60.0), mu)
            .unwrap();
        assert!(just_before.r.norm() < SOI);
    }

    #[test]
    fn circularizing_at_apoapsis_raises_periapsis_to_meet_it() {
        const MU: f64 = 10.0;
        let (a, e) = (10.0, 0.3);
        let orbit = State::from_kepler(a, e, 0.1, 0.2, 0.3, PI, EphemerisTime::epoch(), MU);

        let (circ, dv) = circularization(&orbit, MU);

        assert!(circ.ecc(MU) < 1e-12, "e = {}", circ.ecc(MU));
        assert_eq!(circ.r, orbit.r);
        // same plane, same direction of travel
        let tilt = circ
            .angular_momentum()
            .normalize()
            .dot(&orbit.angular_momentum().normalize());
        assert!((tilt - 1.0).abs() < 1e-12);
        // circular speed at apoapsis, minus the speed there now
        let r_apo = a * (1.0 + e);
        let expected = (MU / r_apo).sqrt() - (MU * (2.0 / r_apo - 1.0 / a)).sqrt();
        assert!((dv / expected - 1.0).abs() < 1e-9, "dv {dv} vs {expected}");
    }

    #[test]
    fn capture_dv_slows_periapsis_speed_to_circular() {
        let mu = G; // Earth
        let (a, e) = (-4.0, 1.5);
        // anywhere on the way in
        let approach = State::from_kepler(a, e, 0.1, 0.2, 0.3, -1.0, EphemerisTime::epoch(), mu);

        let dv = capture_at_periapsis_dv(approach.r, approach.v, mu);

        let q = a * (1.0 - e);
        let expected = (mu * (1.0 + e) / q).sqrt() - (mu / q).sqrt();
        assert!((dv / expected - 1.0).abs() < 1e-9, "dv {dv} vs {expected}");
    }

    #[test]
    fn aiming_at_the_impact_parameter_gives_the_periapsis() {
        let mu = G; // Earth
        let (r_p, v_inf) = (2.0, 20_000.0); // ER, ER/yr (~4 km/s)
        let b = impact_parameter(r_p, v_inf, mu);

        // fly in from far away, offset sideways so the asymptote misses by b:
        // speed from energy conservation, offset so h = b * v_inf exactly
        let far = 1e6;
        let v = (v_inf * v_inf + 2.0 * mu / far).sqrt();
        let approach = State {
            r: DVec3::new(-far, b * v_inf / v, 0.0),
            v: DVec3::new(v, 0.0, 0.0),
            t: EphemerisTime::epoch(),
        };

        let (_, peri) = approach.apsides(mu);
        assert!((peri / r_p - 1.0).abs() < 1e-6, "periapsis {peri} vs {r_p}");
    }

    #[test]
    fn leaving_the_soi_hands_over_to_the_parents_frame() {
        let earth_mu = G;
        const SOI: f64 = 145.0;
        let t0 = EphemerisTime::epoch();
        let earth = State::circular(EARTH_RADII_PER_AU, t0, SUN_MU);
        let (a, e) = (-4.0, 1.5);
        let craft = State::from_kepler(a, e, 0.1, 0.2, 0.3, 0.0, t0, earth_mu);

        let out = get_grandparent_state(&craft, &earth, SOI, SUN_MU, earth_mu).unwrap();

        // relative to Earth, the craft is on the SOI boundary...
        let earth_then = earth.propagate(out.t, SUN_MU).unwrap();
        let r_rel = out.r - earth_then.r;
        assert!(
            (r_rel.norm() / SOI - 1.0).abs() < 1e-6,
            "{} from Earth",
            r_rel.norm()
        );
        // ...moving at the speed energy conservation gives it there: v^2 = mu (2/r - 1/a)
        let v_rel = (out.v - earth_then.v).norm();
        let expected = (earth_mu * (2.0 / SOI - 1.0 / a)).sqrt();
        assert!(
            (v_rel / expected - 1.0).abs() < 1e-6,
            "{v_rel} vs {expected}"
        );
    }

    #[test]
    fn outbound_hyperbola_has_no_periapsis_ahead() {
        let mu = G;
        let t0 = EphemerisTime::epoch();
        // already past the periapsis, so where we are now is technically it
        let orbit = State::from_kepler(-4.0, 1.5, 0.1, 0.2, 0.3, 1.0, t0, mu);

        let peri = find_periapsis(&orbit, t0, mu).unwrap();

        assert!(peri.t > t0);
        assert!(peri.r.dot(&peri.v) > 0.0, "should still be outbound");
    }

    #[test]
    fn circular_orbits_report_a_point_just_ahead() {
        const MU: f64 = 10.0;
        let t0 = EphemerisTime::epoch();
        let orbit = State::circular(10.0, t0, MU);

        // every point on a circle is both apsides, so any upcoming point will do
        let peri = find_periapsis(&orbit, t0, MU).unwrap();
        let apo = find_apoapsis(&orbit, t0, MU).unwrap();

        for s in [peri, apo] {
            assert!(s.t > t0);
            assert!((s.r.norm() / 10.0 - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn captured_craft_never_leaves_the_soi() {
        let mu = G;
        let orbit = State::circular(10.0, EphemerisTime::epoch(), mu);
        assert!(find_soi_exit(&orbit, 145.0, mu).is_err());
    }

    #[test]
    fn distant_hyperbolic_periapsis_is_out_of_range() {
        const MU: f64 = 10.0;
        let t0 = EphemerisTime::epoch();
        // periapsis is far past the 10 year search horizon
        let orbit = State::from_kepler(-10.0, 1.5, 0.1, 0.2, 0.3, -2.2, t0, MU);
        assert!(find_periapsis(&orbit, t0, MU).is_err());
    }
}
