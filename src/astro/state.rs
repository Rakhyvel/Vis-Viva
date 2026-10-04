use std::f64::consts::PI;

use nalgebra_glm::{quat_angle_axis, quat_rotate_vec3, vec3, DVec3};

use crate::astro::{epoch::EphemerisTime, stumpff::stumpff_c2_c3, units::SECONDS_PER_YEAR};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub r: DVec3,
    pub v: DVec3,
    pub t: EphemerisTime,
}

impl State {
    /// Create a state along a circular orbit, at some orbital radius `r`, time `t`, and with the parent grav
    /// param `mu`.
    pub fn circular(r: f64, t: EphemerisTime, mu: f64) -> Self {
        Self::from_kepler(r, 0.0, 0.0, 0.0, 0.0, 0.0, t, mu)
    }

    /// Create a state from Keplerian elements.
    ///
    /// If the orbit is non-elliptical,
    #[allow(clippy::too_many_arguments)] // TODO: Kepler struct?
    pub fn from_kepler(
        a: f64,
        e: f64,
        i: f64,
        raan: f64,
        arg_peri: f64,
        true_anomaly: f64,
        t: EphemerisTime,
        mu: f64,
    ) -> Self {
        // Position in perifocal frame
        let p = a * (1.0 - e * e);
        assert!(
            p > 0.0,
            "semi-latus rectum must be positive, a > 0 for ellipses, a < 0 for hyperbolas (a={a}, e={e})"
        );
        assert!(
            1.0 + e * true_anomaly.cos() > 0.0,
            "true anomaly {true_anomaly} is past the hyperbola's asymptotes (e={e})"
        );

        let r_p = p / (1.0 + e * true_anomaly.cos());
        let r_pf = vec3(r_p * true_anomaly.cos(), r_p * true_anomaly.sin(), 0.0);

        // Velocity in perifocal frame
        let h = (mu * p).sqrt();
        let v_pf = vec3(
            -mu / h * true_anomaly.sin(),
            mu / h * (e + true_anomaly.cos()),
            0.0,
        );

        // Rotate to inertial frame
        let q_raan = quat_angle_axis(raan, &vec3(0.0, 0.0, 1.0));
        let q_i = quat_angle_axis(i, &vec3(1.0, 0.0, 0.0));
        let q_argp = quat_angle_axis(arg_peri, &vec3(0.0, 0.0, 1.0));
        let q = q_raan * q_i * q_argp;

        let r = quat_rotate_vec3(&q, &r_pf);
        let v = quat_rotate_vec3(&q, &v_pf);

        Self { r, v, t }
    }

    /// Returns the ephemeris at some `t` given some mu
    /// TODO:
    /// * Splitting into newton iteration with a fallback, lagrange coefficients would make it easier to test
    /// * bracket/bisection
    pub fn propagate(&self, t: EphemerisTime, mu: f64) -> Result<State, String> {
        let dt = (t - self.t).as_years();

        if !mu.is_finite() || mu <= 0.0 {
            return Err("mu must be finite and positive".into());
        }

        let r0_mag = self.r.norm();
        let v0_mag = self.v.norm();

        if !self.r.iter().all(|x| x.is_finite()) || !self.v.iter().all(|x| x.is_finite()) {
            return Err("state contains non-finite values".into());
        }

        if r0_mag == 0.0 {
            return Err("position magnitude must be non-zero".into());
        }

        let vr0 = self.r.dot(&self.v) / r0_mag; // radial velocity
        let alpha = 2.0 / r0_mag - (v0_mag * v0_mag) / mu; // 1/a (specific energy form)

        // Setup chi with an initial guess
        let mut chi = initial_chi(r0_mag, vr0, alpha, dt, mu);

        let mut best = (f64::INFINITY, chi);

        // newton rhapson, find chi that satisfies the EOM for our given dt
        const MAX_ITER: usize = 500;
        let f_tol = 1e-13 * (mu.sqrt() * dt).abs().max(1.0);
        let mut converged = false;
        for _ in 0..MAX_ITER {
            let chi2 = chi * chi;
            let z = alpha * chi2;

            let (c, s) = stumpff_c2_c3(z);

            // how initial radial motion affects dt
            let r0_vr0_over_sqrtmu = r0_mag * vr0 / mu.sqrt();

            // residual to drive to 0, how far `chi` is to satisfying the EOM
            let f = r0_vr0_over_sqrtmu * chi2 * c
                + (1.0 - alpha * r0_mag) * chi2 * chi * s
                + r0_mag * chi
                - (mu.sqrt() * dt);

            if !f.is_finite() {
                return Err(String::from("universal kepler equation diverged"));
            }

            if f.abs() < best.0 {
                best = (f.abs(), chi);
            }

            if f.abs() < f_tol {
                converged = true;
                break;
            }

            // derivative of F wrt chi, roughly how to nudge chi based on our error to reduce errors to 0
            let df_dchi = r0_vr0_over_sqrtmu * chi * (1.0 - alpha * chi2 * s)
                + (1.0 - alpha * r0_mag) * chi2 * c
                + r0_mag;

            if !df_dchi.is_finite() || df_dchi == 0.0 {
                return Err(String::from("universal kepler derivative became invalid"));
            }

            // Make the delta proportional so large chi can actually move
            let chi_scale = (mu.sqrt() * dt.abs() / r0_mag).max(r0_mag.sqrt());
            let max_step = (0.5 * chi.abs()).max(1e-3 * chi_scale);
            let delta = (f / df_dchi).clamp(-max_step, max_step);

            // make the newton step, towards where 0 residuals are
            chi -= delta;
        }

        if !converged {
            // Accept a good enough answer rather than failing outright
            const SLOP: f64 = 1e4;
            if best.0 < f_tol * SLOP {
                chi = best.1;
            } else {
                return Err(format!(
                    "universal kepler equation did not converge, best: (f={}, chi={})",
                    best.0, best.1
                ));
            }
        }

        let chi2 = chi * chi;

        let z = alpha * chi2;
        let (c, s) = stumpff_c2_c3(z);

        // find lagrange f and g coeffs
        let f = 1.0 - (chi2 / r0_mag) * c;
        let g = dt - (1.0 / mu.sqrt()) * chi2 * chi * s;

        // position at t
        let r = self.r * f + self.v * g;
        let r_mag = r.norm();

        // derive fdot from position rather than chi for better numerical stability
        let fdot = (mu.sqrt() / (r_mag * r0_mag)) * chi * (z * s - 1.0);
        let gdot = 1.0 - (chi2 / r_mag) * c;

        let v = self.r * fdot + self.v * gdot;

        Ok(State { r, v, t })
    }

    pub fn generate_orbit_vertices(
        &self,
        segments: usize,
        mu: f64,
        soi_radius: Option<f64>,
    ) -> Result<Vec<DVec3>, String> {
        // Find period, if periodic, do the loop
        // If not periodic, clip to SOI

        let mut vertices = Vec::with_capacity(segments + 1);

        if let Some(period) = self.period(mu) {
            let step = EphemerisTime::from_secs(period * SECONDS_PER_YEAR / segments as f64);
            let mut et = self.t;
            for _ in 0..=segments {
                vertices.push(self.propagate(et, mu)?.r);
                et += step;
            }
        } else {
            let soi_radius = soi_radius.ok_or("must specify a SOI radius for hyperbolic orbits")?;

            // Use the perifocal frame
            let e_vec = self.ecc_vector(mu);
            let e = e_vec.norm();
            let h = self.angular_momentum();
            let p = h.norm_squared() / mu;
            let p_hat = e_vec / e;
            let q_hat = h.normalize().cross(&p_hat);

            // figure out the nu where the hyperbola exits the SOI
            let nu_exit = ((p / soi_radius - 1.0) / e).clamp(-1.0, 1.0).acos();

            let nu_now = self.true_anomaly(mu);
            // inbound is negative
            let nu_now = if nu_now > PI {
                nu_now - 2.0 * PI
            } else {
                nu_now
            };

            if nu_now >= nu_exit {
                // already leaving the SOI
                return Ok(vec![self.r]);
            }

            // even steps in true anomaly has the effect that the periapsis has more detail than the asymtotes
            // thats what we want!!
            for i in 0..=segments {
                let nu =
                    nu_now + (nu_exit - nu_now) * f64::from(i as i32) / f64::from(segments as i32);
                // get the points straight from the conic equation rather than calling newpton each time
                let r = p / (1.0 + e * nu.cos());
                vertices.push(r * (nu.cos() * p_hat + nu.sin() * q_hat));
            }
        }

        Ok(vertices)
    }

    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        -mu / (2.0 * self.specific_energy(mu))
    }

    /// Points from the focus to the periapsis, length equal to the ecc
    pub fn ecc_vector(&self, mu: f64) -> DVec3 {
        let r_mag = self.r.norm();
        let h = self.angular_momentum();
        self.v.cross(&h) / mu - self.r / r_mag
    }

    pub fn ecc(&self, mu: f64) -> f64 {
        self.ecc_vector(mu).norm()
    }

    /// Get the inclination from the +Z axis, in radians
    pub fn inclination(&self) -> f64 {
        // find angular momentum vec
        let h_vec = self.angular_momentum();
        let h_mag = h_vec.norm();

        if h_mag == 0.0 {
            return 0.0;
        }

        // assume +z is the axis
        let cos_i = (h_vec.z / h_mag).clamp(-1.0, 1.0);

        cos_i.acos()
    }

    /// Get the (apoapsis, periapsis), in ER
    pub fn apsides(&self, mu: f64) -> (Option<f64>, f64) {
        let h = self.angular_momentum().norm();
        let p = h * h / mu;
        let e = self.ecc(mu);
        let peri = p / (1.0 + e);
        let apo = (e < 1.0).then(|| p / (1.0 - e));
        (apo, peri)
    }

    pub fn true_anomaly(&self, mu: f64) -> f64 {
        let r = self.r;

        let h = self.angular_momentum();

        let e_vec = self.ecc_vector(mu);
        let e = e_vec.norm();

        // If eccentricity is near 0, orbit is near-circular and true anomaly is undefined (no periapsis!)
        // Fallback to argument of latitude
        const EPS: f64 = 1e-6;
        if e < EPS {
            let k = DVec3::z();
            let n = k.cross(&h);
            let n_mag = n.norm();
            // Inclined
            if n_mag > EPS {
                let u = r.dot(&h.normalize().cross(&n)).atan2(r.dot(&n));
                return u.rem_euclid(2.0 * PI);
            }
            // Equatorial
            else {
                let lambda = r.y.atan2(r.x);
                return lambda.rem_euclid(2.0 * PI);
            }
        }

        // Regular case for elliptical orbits
        let nu = e_vec.cross(&r).dot(&h).atan2(e_vec.dot(&r) * h.norm());
        nu.rem_euclid(2.0 * PI)
    }

    pub fn mean_anomaly(&self, mu: f64) -> f64 {
        let e = self.ecc(mu);
        assert!(
            e < 1.0,
            "mean anomaly is only implemented for ellipses (e={e})"
        );
        let nu = self.true_anomaly(mu);

        // Eccentric anomaly from true anomaly
        let cos_ea = (e + nu.cos()) / (1.0 + e * nu.cos());
        let sin_ea = (1.0 - e * e).sqrt() * nu.sin() / (1.0 + e * nu.cos());
        let ea = sin_ea.atan2(cos_ea);

        // Mean anomaly from eccentric anomaly (Kepler's equation)
        (ea - e * ea.sin()).rem_euclid(2.0 * PI)
    }

    /// Returns the period in Earth years of the orbit is periodic, otherwise returns None
    pub fn period(&self, mu: f64) -> Option<f64> {
        let a = self.semi_major_axis(mu);

        if a <= 0.0 {
            // a < 0 -> hyperbolic, a = 0 -> parabolic (1/a = 0 means v = escape velocity)
            return None;
        }

        let period = 2.0 * PI * (a.powi(3) / mu).sqrt();

        // Treat near-parabolic orbits as hyperbolic to avoid overflow
        // 1000 years is already unrenderable
        if period > 1000.0 {
            return None;
        }

        Some(period)
    }

    /// Angular momentum per unit mass
    pub fn angular_momentum(&self) -> DVec3 {
        self.r.cross(&self.v)
    }

    pub fn specific_energy(&self, mu: f64) -> f64 {
        self.v.norm_squared() / 2.0 - mu / self.r.norm()
    }
}

fn initial_chi(r0_mag: f64, vr0: f64, alpha: f64, dt: f64, mu: f64) -> f64 {
    if alpha > 0.0 {
        // Elliptic, seed with circular orbit chi
        mu.sqrt() * dt * alpha
    } else if alpha.abs() > 1e-12 {
        // Hyperbolic, seed using Vallado alg. 8
        let a = 1.0 / alpha;
        let sign = if dt < 0.0 { -1.0 } else { 1.0 };
        let num = -2.0 * mu * alpha * dt;
        let den = r0_mag * vr0 + sign * (-mu * a).sqrt() * (1.0 - r0_mag * alpha);
        let ratio = num / den;
        if ratio > 0.0 && ratio.is_finite() {
            sign * (-a).sqrt() * ratio.ln()
        } else {
            mu.sqrt() * dt / r0_mag
        }
    } else {
        // Near parabolic
        mu.sqrt() * dt / r0_mag
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    #[track_caller]
    fn assert_close(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64) {
        let abs_err = (actual - expected).abs();
        let rel_err = abs_err / expected.abs().max(f64::MIN_POSITIVE);
        assert!(
            abs_err < abs_tol || rel_err < rel_tol,
            "expected {}, got {} (abs_err={}, rel_err={})",
            expected,
            actual,
            abs_err,
            rel_err
        );
    }

    #[track_caller]
    fn assert_close_tight(actual: f64, expected: f64) {
        assert_close(actual, expected, 1e-14, 1e-14);
    }

    #[track_caller]
    fn assert_close_loose(actual: f64, expected: f64) {
        assert_close(actual, expected, 1e-8, 1e-8);
    }

    /// Angles are equal if they differ by a multiple of 2pi
    #[track_caller]
    fn assert_angle_close(actual: f64, expected: f64) {
        // wrap the difference into [-pi, pi)
        let diff = (actual - expected + PI).rem_euclid(2.0 * PI) - PI;
        assert!(
            diff.abs() < 1e-12,
            "expected angle {expected}, got {actual} (diff={diff})"
        );
    }

    #[test]
    fn circular() {
        const RADIUS: f64 = 10.0;
        const MU: f64 = 2.0;
        let state = State::circular(RADIUS, EphemerisTime::epoch(), MU);

        assert_close_tight(state.r.norm(), RADIUS);
        assert_close_tight(state.v.norm(), (MU / RADIUS).sqrt());
        assert_close_tight(state.r.dot(&state.v), 0.0);
        assert_close_tight(state.inclination(), 0.0);
        assert_close_tight(state.ecc(MU), 0.0);
        assert_close_tight(
            state.period(MU).unwrap(),
            2.0 * PI * (RADIUS.powi(3) / MU).sqrt(),
        );
    }

    #[test]
    fn circular_orbits_measure_from_the_node_or_x_axis() {
        const MU: f64 = 10.0;
        // no periapsis, so inclined orbits report the argument of latitude, arg_peri + nu
        let inclined =
            State::from_kepler(10.0, 0.0, 0.5, 0.3, 0.2, 3.8, EphemerisTime::epoch(), MU);
        assert_angle_close(inclined.true_anomaly(MU), 4.0);
        // and equatorial ones have no node either, so the true longitude, raan + arg_peri + nu
        let equatorial =
            State::from_kepler(10.0, 0.0, 0.0, 0.3, 0.2, 3.8, EphemerisTime::epoch(), MU);
        assert_angle_close(equatorial.true_anomaly(MU), 4.3);
    }

    #[test]
    fn keplerian_round_trip() {
        const SEMI_MAJOR_AXIS: f64 = 10.0;
        const ECCENTRICITY: f64 = 0.3;
        const INCLINATION: f64 = 0.4;
        const RAAN: f64 = 0.5;
        const ARG_PERI: f64 = 0.6;
        const TRUE_ANOMALY: f64 = 0.7;
        const MU: f64 = 2.0;

        let state = State::from_kepler(
            SEMI_MAJOR_AXIS,
            ECCENTRICITY,
            INCLINATION,
            RAAN,
            ARG_PERI,
            TRUE_ANOMALY,
            EphemerisTime::epoch(),
            MU,
        );

        assert_close_tight(state.semi_major_axis(MU), SEMI_MAJOR_AXIS);
        assert_close_tight(state.ecc(MU), ECCENTRICITY);
        assert_close_tight(state.inclination(), INCLINATION);
        assert_close_tight(state.true_anomaly(MU), TRUE_ANOMALY);
    }

    #[test]
    #[should_panic(expected = "a < 0 for hyperbolas")]
    fn hyperbola_with_positive_a_is_rejected() {
        State::from_kepler(10.0, 1.3, 0.4, 0.5, 0.6, 0.7, EphemerisTime::epoch(), 2.0);
    }

    #[test]
    fn hyperbolic_round_trip() {
        const SEMI_MAJOR_AXIS: f64 = -10.0;
        const ECCENTRICITY: f64 = 1.3;
        const INCLINATION: f64 = 0.4;
        const RAAN: f64 = 0.5;
        const ARG_PERI: f64 = 0.6;
        const TRUE_ANOMALY: f64 = 0.7;
        const MU: f64 = 2.0;

        let state = State::from_kepler(
            SEMI_MAJOR_AXIS,
            ECCENTRICITY,
            INCLINATION,
            RAAN,
            ARG_PERI,
            TRUE_ANOMALY,
            EphemerisTime::epoch(),
            MU,
        );

        assert_close_tight(state.semi_major_axis(MU), SEMI_MAJOR_AXIS);
        assert_close_tight(state.ecc(MU), ECCENTRICITY);
        assert_close_tight(state.inclination(), INCLINATION);
        assert_close_tight(state.true_anomaly(MU), TRUE_ANOMALY);

        assert!(state.period(MU).is_none());
        assert!(state.apsides(MU).0.is_none())
    }

    #[test]
    fn true_anomaly_at_apsides() {
        const MU: f64 = 10.0;
        // periapsis, apoapsis, and points either side of each, where acos is worst
        for nu in [0.0, PI, 1e-9, -1e-9, PI - 1e-9, PI + 1e-9, 1.0, 4.0] {
            let state =
                State::from_kepler(10.0, 0.1, 0.1, 0.0, 0.0, nu, EphemerisTime::epoch(), MU);
            assert_angle_close(state.true_anomaly(MU), nu);
        }
    }

    #[test]
    fn no_period_when_unbound_or_too_slow() {
        let parabola = State {
            r: DVec3::new(2.0, 0.0, 0.0),
            v: DVec3::new(0.0, 1.0, 0.0),
            t: EphemerisTime::epoch(),
        };
        assert_eq!(parabola.ecc(1.0), 1.0);
        assert_eq!(parabola.apsides(1.0), (None, 2.0));
        assert!(parabola.period(1.0).is_none());

        // bound, but its ~22,000 year period is too long to draw
        let wide = State::circular(500.0, EphemerisTime::epoch(), 10.0);
        assert!(wide.apsides(10.0).0.is_some());
        assert!(wide.period(10.0).is_none());
    }

    #[test]
    fn zero_propagation() {
        const MU: f64 = 10.0;
        let state_1 = State::from_kepler(10.0, 0.1, 0.1, 0.0, 0.0, 0.0, EphemerisTime::epoch(), MU);
        let state_2 = state_1.propagate(state_1.t, MU).unwrap();

        assert_close_loose(state_1.r.norm() - state_2.r.norm(), 0.0);
        assert_close_loose(state_1.v.norm() - state_2.v.norm(), 0.0);
        assert_close_loose(state_1.t.as_secs(), state_2.t.as_secs());
    }

    #[test]
    fn one_rev_propagation() {
        const MU: f64 = 10.0;
        let state_1 = State::from_kepler(10.0, 0.1, 0.1, 0.0, 0.0, 0.0, EphemerisTime::epoch(), MU);
        let period = state_1.period(MU).unwrap();
        let target = state_1.t + EphemerisTime::from_years(period);
        let state_2 = state_1.propagate(target, MU).unwrap();

        assert_close_loose(state_1.r.norm() - state_2.r.norm(), 0.0);
        assert_close_loose(state_1.v.norm() - state_2.v.norm(), 0.0);
        assert_eq!(state_2.t, target);
    }

    #[test]
    fn forward_backward_propagation() {
        const MU: f64 = 10.0;
        let state_1 = State::from_kepler(10.0, 0.1, 0.1, 0.0, 0.0, 0.0, EphemerisTime::epoch(), MU);
        let state_2 = state_1
            .propagate(state_1.t + EphemerisTime::from_days(30.0), MU)
            .unwrap();
        let state_3 = state_2.propagate(state_1.t, MU).unwrap();

        assert_close_tight(state_1.r.norm() - state_3.r.norm(), 0.0);
        assert_close_tight(state_1.v.norm() - state_3.v.norm(), 0.0);
        assert_close_tight(state_1.t.as_secs(), state_3.t.as_secs());
    }

    /// An elliptical, an eccentric, and a hyperbolic orbit, propagated across many orbits
    /// and backwards
    fn conservation_cases() -> impl Iterator<Item = (State, State)> {
        const MU: f64 = 10.0;
        let orbits = [
            State::from_kepler(10.0, 0.1, 0.1, 0.2, 0.3, 0.4, EphemerisTime::epoch(), MU),
            State::from_kepler(10.0, 0.9, 0.1, 0.2, 0.3, 0.4, EphemerisTime::epoch(), MU),
            State::from_kepler(-10.0, 1.5, 0.1, 0.2, 0.3, 0.4, EphemerisTime::epoch(), MU),
        ];
        orbits.into_iter().flat_map(|s| {
            [0.1, 1.0, 10.0, 100.0, 500.0, -3.0].map(move |years| {
                (
                    s,
                    s.propagate(s.t + EphemerisTime::from_years(years), MU)
                        .unwrap(),
                )
            })
        })
    }

    #[test]
    fn propagation_conserves_energy() {
        const MU: f64 = 10.0;
        for (start, end) in conservation_cases() {
            let e0 = start.specific_energy(MU);
            let drift = (end.specific_energy(MU) - e0) / e0;
            assert_close_loose(drift, 0.0);
        }
    }

    #[test]
    fn propagation_conserves_angular_momentum() {
        for (start, end) in conservation_cases() {
            let h0 = start.angular_momentum();
            // compares direction too, so the orbital plane can't tilt
            let drift = (end.angular_momentum() - h0).norm() / h0.norm();
            assert_close_loose(drift, 0.0);
        }
    }

    #[test]
    fn mean_anomaly_advances_uniformly() {
        const MU: f64 = 10.0;
        const A: f64 = 10.0;
        // mean motion from Kepler's third law, independent of period()
        let n = (MU / A.powi(3)).sqrt();
        for e in [0.1, 0.7, 0.95] {
            let start = State::from_kepler(A, e, 0.1, 0.2, 0.3, 0.4, EphemerisTime::epoch(), MU);
            let m0 = start.mean_anomaly(MU);

            // do many periods
            for years in [0.5, 7.0, 31.0, 100.0, 500.0, -12.0] {
                let end = start
                    .propagate(start.t + EphemerisTime::from_years(years), MU)
                    .unwrap();
                // wrap into [-pi, pi)
                let diff = (end.mean_anomaly(MU) - (m0 + n * years) + PI).rem_euclid(2.0 * PI) - PI;
                assert!(diff.abs() < 1e-11, "e={e}, {years} years: off by {diff}");
            }
        }

        // short steps at Earth scale, starting at periapsis where Newton's first guess is worst
        let (mu, a) = (1.5e9, 2.0_f64);
        let n = (mu / a.powi(3)).sqrt();
        let start = State::from_kepler(a, 0.5, 0.1, 0.2, 0.3, 0.0, EphemerisTime::epoch(), mu);
        let m0 = start.mean_anomaly(mu);
        for secs in [1.0, 60.0, 3600.0] {
            let end = start
                .propagate(start.t + EphemerisTime::from_secs(secs), mu)
                .unwrap();
            let expected = m0 + n * secs / SECONDS_PER_YEAR;
            let diff = (end.mean_anomaly(mu) - expected + PI).rem_euclid(2.0 * PI) - PI;
            assert!(diff.abs() < 1e-11, "{secs} s: off by {diff}");
        }
    }

    #[test]
    fn propagate_rejects_bad_mu() {
        let state = State::from_kepler(10.0, 0.1, 0.1, 0.2, 0.3, 0.4, EphemerisTime::epoch(), 10.0);

        assert_eq!(
            state
                .propagate(state.t + EphemerisTime::from_days(30.0), -30.0)
                .err()
                .unwrap(),
            String::from("mu must be finite and positive")
        );

        assert_eq!(
            state
                .propagate(state.t + EphemerisTime::from_days(30.0), f64::NAN)
                .err()
                .unwrap(),
            String::from("mu must be finite and positive")
        );
    }

    #[test]
    fn propagate_rejects_bad_radial() {
        let state = State {
            r: vec3(f64::NAN, f64::NAN, f64::NAN),
            v: vec3(0.0, 0.0, 0.0),
            t: EphemerisTime::epoch(),
        };

        assert_eq!(
            state
                .propagate(state.t + EphemerisTime::from_days(30.0), 10.0)
                .err()
                .unwrap(),
            String::from("state contains non-finite values")
        );
    }

    #[test]
    fn propagate_rejects_bad_velocity() {
        let state = State {
            r: vec3(0.0, 0.0, 0.0),
            v: vec3(f64::NAN, f64::NAN, f64::NAN),
            t: EphemerisTime::epoch(),
        };

        assert_eq!(
            state
                .propagate(state.t + EphemerisTime::from_days(30.0), 10.0)
                .err()
                .unwrap(),
            String::from("state contains non-finite values")
        );
    }

    #[test]
    fn orbit_vertices_form_closed_loop() {
        const MU: f64 = 10.0;
        const SEGMENTS: usize = 64;
        // start at periapsis, so half a period later is apoapsis
        let start = State::from_kepler(10.0, 0.5, 0.1, 0.2, 0.3, 0.0, EphemerisTime::epoch(), MU);
        let (apo, peri) = start.apsides(MU);
        let apo = apo.unwrap();
        let normal = start.angular_momentum().normalize();

        let vertices = start.generate_orbit_vertices(SEGMENTS, MU, None).unwrap();

        assert_eq!(vertices.len(), SEGMENTS + 1);
        assert_close_tight((vertices[0] - start.r).norm(), 0.0);
        assert_close_loose((vertices[SEGMENTS] - vertices[0]).norm(), 0.0);
        assert_close_loose(vertices[SEGMENTS / 2].norm(), apo);
        for v in &vertices {
            assert_close_tight(v.dot(&normal), 0.0);
            assert!(
                peri - 1e-9 <= v.norm() && v.norm() <= apo + 1e-9,
                "vertex off the orbit: {v:?}"
            );
        }
    }

    #[test]
    fn hyperbolic_vertices_stop_at_soi() {
        const MU: f64 = 1.5e9; // ~Earth, in ER^3/yr^2
        const SOI: f64 = 145.0; // ~Earth's SOI, in ER
                                // periapsis 2 ER (a = -q / (e - 1)), starting inbound so the line passes periapsis
        const SEGMENTS: usize = 64;
        // periapsis 2 ER, starting inbound so the line passes periapsis
        let start = State::from_kepler(-4.0, 1.5, 0.1, 0.2, 0.3, -1.0, EphemerisTime::epoch(), MU);
        let (_, peri) = start.apsides(MU);
        let normal = start.angular_momentum().normalize();

        let vertices = start
            .generate_orbit_vertices(SEGMENTS, MU, Some(SOI))
            .unwrap();

        assert_eq!(vertices.len(), SEGMENTS + 1);
        assert_close_loose((vertices[0] - start.r).norm(), 0.0);
        assert_close_loose(vertices[SEGMENTS].norm(), SOI);
        for v in &vertices {
            assert_close_loose(v.dot(&normal), 0.0);
            assert!(
                peri - 1e-9 <= v.norm() && v.norm() <= SOI + 1e-9,
                "vertex off the orbit: {v:?}"
            );
        }
        // vertices bunch up around periapsis, so the line gets close to it
        let closest = vertices
            .iter()
            .map(|v| v.norm())
            .fold(f64::INFINITY, f64::min);
        assert!(
            closest - peri < 1e-3,
            "closest vertex {closest} vs periapsis {peri}"
        );
    }
}
