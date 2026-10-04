use nalgebra_glm::DVec3;

use rayon::iter::{IntoParallelIterator, ParallelIterator};

use std::f64::consts::PI;

use crate::astro::{epoch::EphemerisTime, lambert::TransferKind, state::State};

pub struct Porkchop {
    pub current_et: EphemerisTime,
    pub step: EphemerisTime,
    pub depart_steps: usize,
    pub tof_min: f64,
    pub tof_max: f64,
    pub tof_steps: usize,
    /// tof-major, index = j * depart_steps + i, row 0 = longest TOF
    pub cells: Vec<Option<Cell>>,
}

pub struct Cell {
    pub depart_dv: DVec3,
    pub arrival_dv: f64,
    pub total: f64,
}

impl Porkchop {
    pub fn compute<F>(w: &SweepWindow, depart_steps: usize, tof_steps: usize, eval: F) -> Self
    where
        F: Fn(EphemerisTime, f64) -> Option<Cell> + Sync,
    {
        assert!(
            depart_steps >= 1 && tof_steps >= 2,
            "porkchop needs at least 1 col and 2 rows"
        );

        let step = EphemerisTime::from_years(w.sweep / depart_steps as f64);

        let cells: Vec<Option<Cell>> = (0..tof_steps)
            .flat_map(|j| (0..depart_steps).map(move |i| (i, j)))
            .collect::<Vec<_>>() // TODO: A range here might be simpler
            .into_par_iter()
            .map(|(i, j)| {
                let tof = Self::tof_for_row(w.tof_min, w.tof_max, tof_steps, j);
                eval(w.start + step * i as i64, tof)
            })
            .collect();

        Self {
            current_et: w.start,
            step,
            depart_steps,
            tof_min: w.tof_min,
            tof_max: w.tof_max,
            tof_steps,
            cells,
        }
    }

    pub fn depart_at(&self, i: usize) -> EphemerisTime {
        self.current_et + self.step * i as i64
    }

    pub fn tof_at(&self, j: usize) -> f64 {
        Self::tof_for_row(self.tof_min, self.tof_max, self.tof_steps, j)
    }

    pub fn best(&self, objective: &TransferObjective) -> Option<(usize, usize, &Cell)> {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(n, c)| {
                let c = c.as_ref()?;
                let (i, j) = (n % self.depart_steps, n / self.depart_steps);
                let cost = objective.cost(c.total, self.tof_at(j))?;
                Some((i, j, c, cost))
            })
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(i, j, c, _)| (i, j, c))
    }

    pub fn at(&self, i: usize, j: usize) -> Option<&Cell> {
        self.cells.get(j * self.depart_steps + i)?.as_ref()
    }

    fn tof_for_row(tof_min: f64, tof_max: f64, tof_steps: usize, j: usize) -> f64 {
        tof_max - (tof_max - tof_min) * j as f64 / (tof_steps - 1) as f64
    }
}

#[derive(Debug)]
pub enum TransferObjective {
    /// minimize total delta-v
    MinFuel,
    /// minimize tof, subject to a max delta-v budget
    MinTof { max_dv: f64 }, // TODO: should probably be SoonestArrivalTime
    /// weighed combination, alpha * dv + (1 - alpha) * tof
    Balanced { dv_weight: f64, tof_weight: f64 },
}

impl TransferObjective {
    /// return the cost given dv and tof, if feasible
    pub fn cost(&self, dv: f64, tof: f64) -> Option<f64> {
        match self {
            TransferObjective::MinFuel => Some(dv),
            TransferObjective::MinTof { max_dv } => {
                if dv <= *max_dv {
                    Some(tof)
                } else {
                    None
                }
            }
            TransferObjective::Balanced {
                dv_weight,
                tof_weight,
            } => Some(*dv_weight * dv + tof_weight * tof),
        }
    }
}

#[derive(Debug)]
pub struct SweepWindow {
    pub start: EphemerisTime,
    pub sweep: f64,
    // the untruncated synodic period, for clamping
    pub full: f64,
    pub tof_min: f64,
    pub tof_max: f64,
}

pub fn sweep_window(
    craft: &State,
    target: &State,
    mu: f64,
    current_et: EphemerisTime,
) -> Result<SweepWindow, String> {
    let transfer_a = (craft.semi_major_axis(mu) + target.semi_major_axis(mu)) / 2.0;
    let tof_guess = PI * (transfer_a.powi(3) / mu).sqrt();

    let craft_period = craft
        .period(mu)
        .ok_or("can't transfer from a hyperbolic orbit")?;
    let target_period = target
        .period(mu)
        .ok_or("can't transfer to a hyperbolic orbit")?;

    let synodic = 1.0 / (1.0 / craft_period - 1.0 / target_period).abs();

    Ok(SweepWindow {
        start: current_et,
        sweep: synodic.min(craft_period * 20.0),
        full: synodic,
        tof_min: tof_guess / 1.3,
        tof_max: tof_guess * 1.3,
    })
}

pub fn best_branch<F>(f: F) -> Option<Cell>
where
    F: Fn(TransferKind) -> Option<Cell>,
{
    [TransferKind::Short, TransferKind::Long]
        .into_iter()
        .filter_map(f)
        .min_by(|a, b| a.total.total_cmp(&b.total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> SweepWindow {
        SweepWindow {
            start: EphemerisTime::epoch(),
            sweep: 2.0,
            full: 2.0,
            tof_min: 1.0,
            tof_max: 3.0,
        }
    }

    #[test]
    fn grid_spawns_the_window() {
        let w = window();
        let chop = Porkchop::compute(&w, 4, 3, |_, _| None);

        assert_eq!(chop.depart_at(0), w.start);
        assert_eq!(
            chop.depart_at(4),
            w.start + EphemerisTime::from_years(w.sweep)
        );

        // row 0 is the longest tof
        assert_eq!(chop.tof_at(0), w.tof_max);
        assert_eq!(chop.tof_at(2), w.tof_min);
    }

    #[test]
    fn each_cell_is_evaluated_at_its_own_departure_and_flight_time() {
        let w = window();
        // store the inputs, so we can check every cell got its own
        let chop = Porkchop::compute(&w, 4, 3, |et, tof| {
            Some(Cell {
                depart_dv: DVec3::zeros(),
                arrival_dv: tof,
                total: (et - w.start).as_years(),
            })
        });
        for i in 0..4 {
            for j in 0..3 {
                let cell = chop.at(i, j).unwrap();
                assert_eq!(cell.total, (chop.depart_at(i) - w.start).as_years());
                assert_eq!(cell.arrival_dv, chop.tof_at(j));
            }
        }
    }

    #[test]
    fn best_is_the_cheapest_solved_cell() {
        let w = window();
        // departures 0, 0.5, 1, 1.5, flights 3, 2, 1. Cheapest at depart 1, flight 2 (i = 2, j = 1)
        let chop = Porkchop::compute(&w, 4, 3, |et, tof| {
            let depart = (et - w.start).as_years();
            // unsolvable cells must be skipped, not treated as free
            (tof != 3.0).then(|| Cell {
                depart_dv: DVec3::zeros(),
                arrival_dv: 0.0,
                total: (depart - 1.0).powi(2) + (tof - 2.0).powi(2),
            })
        });
        let (i, j, _) = chop.best(&TransferObjective::MinFuel).unwrap();
        assert_eq!((i, j), (2, 1));
    }

    #[test]
    fn window_scales_with_the_orbits() {
        let t0 = EphemerisTime::epoch();
        let window = |k: f64, mu: f64| {
            let craft = State::circular(1.0 * k, t0, mu);
            let target = State::circular(1.5 * k, t0, mu);
            sweep_window(&craft, &target, mu, t0).unwrap()
        };
        let base = window(1.0, 1.0);
        // time should scale with mu
        for (scaled, factor) in [(window(4.0, 1.0), 8.0), (window(1.0, 4.0), 0.5)] {
            for (name, s, b) in [
                ("tof_min", base.tof_min, scaled.tof_min),
                ("tof_max", base.tof_max, scaled.tof_max),
                ("sweep", base.sweep, scaled.sweep),
            ] {
                assert!(
                    (b / s - factor).abs() < 1e-12,
                    "{name}: {b} vs {factor} x {s}"
                );
            }
        }
    }

    #[test]
    fn balanced_cost_rises_with_dv_and_flight_time() {
        let objective = TransferObjective::Balanced {
            dv_weight: 1.0,
            tof_weight: 2.0,
        };
        let base = objective.cost(3.0, 4.0).unwrap();
        assert!(
            objective.cost(3.5, 4.0).unwrap() > base,
            "more dv should cost more"
        );
        assert!(
            objective.cost(3.0, 4.5).unwrap() > base,
            "a longer flight should cost more"
        );
    }

    #[test]
    fn a_zero_weight_ignores_that_quantity() {
        // tof preference chooses quicker over a cheaper one
        let tof_only = TransferObjective::Balanced {
            dv_weight: 0.0,
            tof_weight: 1.0,
        };
        assert!(tof_only.cost(10.0, 2.0).unwrap() < tof_only.cost(1.0, 3.0).unwrap());
        // dv preference chooses cheaper over quicker
        let dv_only = TransferObjective::Balanced {
            dv_weight: 1.0,
            tof_weight: 0.0,
        };
        assert!(dv_only.cost(2.0, 10.0).unwrap() < dv_only.cost(3.0, 1.0).unwrap());
    }

    #[test]
    fn min_tof_rejects_transfers_over_budget() {
        let objective = TransferObjective::MinTof { max_dv: 5.0 };
        assert_eq!(objective.cost(4.0, 2.0), Some(2.0));
        assert_eq!(objective.cost(6.0, 1.0), None);
    }
}
