use std::f64::consts::PI;

use nalgebra_glm::{quat_angle_axis, quat_rotate_vec3, vec3, DVec3};

use crate::astro::{
    epoch::{EphemerisTime, ET_PER_SECOND},
    stumpff::{stumpff_c, stumpff_s},
    units::{SECONDS_PER_HOUR, SECONDS_PER_YEAR},
};

#[derive(Debug, Clone, Copy)]
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
    /// * Tests! That we preserve orbital energy and angular momentum
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
        let mut chi = if alpha > 0.0 {
            // Elliptic: seed with circular orbit chi
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
            // Near-parabolic
            mu.sqrt() * dt / r0_mag
        };

        let mut best = (f64::INFINITY, chi);

        // newton rhapson, find chi that satisfies the EOM for our given dt
        const MAX_ITER: usize = 500;
        let f_tol = 1e-10 * (mu.sqrt() * dt).abs().max(1.0);
        let mut converged = false;
        for _ in 0..MAX_ITER {
            let chi2 = chi * chi;
            let z = alpha * chi2;

            let c = stumpff_c(z);
            let s = stumpff_s(z);

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
        let c = stumpff_c(z);
        let s = stumpff_s(z);

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
        segments: i32,
        mu: f64,
        soi_radius: Option<f64>,
    ) -> Result<Vec<f32>, String> {
        // Find period, if periodic, do the loop
        // If not periodic, clip to SOI

        let mut vertices = Vec::with_capacity((segments as usize + 1) * 3);

        if let Some(period) = self.period(mu) {
            let mut et = self.t;

            for _ in 0..=segments {
                let new_state = self.propagate(et, mu)?;

                vertices.push(new_state.r.x as f32);
                vertices.push(new_state.r.y as f32);
                vertices.push(new_state.r.z as f32);

                let step_secs = period * SECONDS_PER_YEAR / (segments as f64);
                if step_secs > i64::MAX as f64 / ET_PER_SECOND {
                    eprintln!(
                        "WARNING: orbit period too large to render: {} years",
                        period
                    );
                    break;
                }
                et += EphemerisTime::from_secs(step_secs); // TODO: It's possible to overflow here?!
            }
        } else {
            let soi_radius = soi_radius.expect("must specify a SOI radius for hyperbolic orbits");
            let mut closest_dist = f64::INFINITY;
            let mut et = self.t;
            loop {
                let pos = self.propagate(et, mu)?.r;
                let distance = pos.norm();

                if distance > closest_dist {
                    if distance >= soi_radius {
                        println!("{distance} >= {soi_radius}");
                        break;
                    }
                } else {
                    closest_dist = distance;
                }

                assert!(pos.x.is_finite());
                assert!(pos.y.is_finite());
                assert!(pos.z.is_finite());

                vertices.push(pos.x as f32);
                vertices.push(pos.y as f32);
                vertices.push(pos.z as f32);

                et += EphemerisTime::from_secs(SECONDS_PER_HOUR);
            }
        }

        Ok(vertices)
    }

    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        let r_mag = self.r.norm();
        1.0 / (2.0 / r_mag - self.v.norm_squared() / mu)
    }

    pub fn ecc(&self, mu: f64) -> f64 {
        let r_mag = self.r.norm();

        let v_cross_h = self.v.cross(&(self.r.cross(&self.v)));

        let e_vec = (v_cross_h / mu) - (self.r / r_mag);

        e_vec.norm()
    }

    /// Get the inclination from the +Z axis, in radians
    pub fn inclination(&self) -> f64 {
        // find angular momentum vec
        let h_vec = self.r.cross(&self.v);
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
        let h = self.r.cross(&self.v).norm();
        let p = h * h / mu;
        let e = self.ecc(mu);
        let peri = p / (1.0 + e);
        let apo = (e < 1.0).then(|| p / (1.0 - e));
        (apo, peri)
    }

    pub fn true_anomaly(&self, mu: f64) -> f64 {
        let r = self.r;
        let v = self.v;

        let r_mag = r.norm();
        let h = r.cross(&v);

        let e_vec = (v.cross(&h) / mu) - (r / r_mag);
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
                let u = r.dot(&n).atan2(r.dot(&h.cross(&n)));
                return u.rem_euclid(2.0 * PI);
            }
            // Equatorial
            else {
                let lambda = r.y.atan2(r.x);
                return lambda.rem_euclid(2.0 * PI);
            }
        }

        // Regular case for elliptical orbits
        let cos_nu = e_vec.dot(&r) / (e * r_mag);
        let nu = cos_nu.clamp(-1.0, 1.0).acos();

        if r.dot(&v) < 0.0 {
            2.0 * PI - nu
        } else {
            nu
        }
    }

    pub fn mean_anomaly(&self, mu: f64) -> f64 {
        let e = self.ecc(mu);
        let nu = self.true_anomaly(mu);

        // Eccentric anomaly from true anomaly
        let cos_ea = (e + nu.cos()) / (1.0 + e * nu.cos());
        let sin_ea = (1.0 - e * e).sqrt() * nu.sin() / (1.0 + e * nu.cos());
        let ea = sin_ea.atan2(cos_ea);

        // Mean anomaly from eccentric anomaly (Kepler's equation)
        (ea - e * ea.sin()).rem_euclid(2.0 * PI)
    }

    /// Returns the period of the orbit is periodic, otherwise returns None
    pub fn period(&self, mu: f64) -> Option<f64> {
        let r = self.r.norm();
        let v2 = self.v.dot(&self.v);
        let a = 1.0 / (2.0 / r - v2 / mu); // semi-major axis from vis-viva
                                           // hey thats a really cool name someone should name a game that

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
}
