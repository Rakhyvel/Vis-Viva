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
    if orbit.ecc(mu) < TOL {
        // orbit is circular, periapsis isn't defined. Just pick this point
        return orbit.propagate(current_et + EphemerisTime::from_secs(60.0), mu);
    }

    if orbit.ecc(mu) >= 1.0 && rdotv_at_t(current_et)? > 0.0 {
        // hyperbolic orbit and we're already past the periapsis
        return orbit.propagate(current_et + EphemerisTime::from_secs(60.0), mu);
    }

    // Use period-based step for elliptical, or r/v based step for hyperbolic
    let dt = if let Some(period) = orbit.period(mu) {
        EphemerisTime::from_years(period / 100.0) // 100 steps per orbit
    } else {
        // Hyperbolic - use time to travel one radius at current speed
        let r = orbit.propagate(current_et, mu)?.r.norm();
        let v = orbit.propagate(current_et, mu)?.v.norm();
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
            return orbit.propagate(current_et, mu);
        }
    }

    // Now march hi forward until rdotv goes positive
    while rdotv_at_t(hi)? < 0.0 {
        hi += dt;
        if hi > max_et {
            return orbit.propagate(current_et, mu);
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
