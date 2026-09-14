use nalgebra_glm::DVec3;

use crate::astro::{departure::TransferObjective, epoch::EphemerisTime};
use rayon::iter::{IntoParallelIterator, ParallelIterator};

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
    pub fn compute<F>(
        current_et: EphemerisTime,
        sweep: f64,
        tof_min: f64,
        tof_max: f64,
        depart_steps: usize,
        tof_steps: usize,
        eval: F,
    ) -> Self
    where
        F: Fn(EphemerisTime, f64) -> Option<Cell> + Sync,
    {
        let step = EphemerisTime::from_years(sweep / depart_steps as f64);

        let cells: Vec<Option<Cell>> = (0..tof_steps)
            .flat_map(|j| (0..depart_steps).map(move |i| (i, j)))
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|(i, j)| {
                let tof = Self::tof_for_row(tof_min, tof_max, tof_steps, j);
                eval(current_et + step * i as i64, tof)
            })
            .collect();

        Self {
            current_et,
            step,
            depart_steps,
            tof_min,
            tof_max,
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
