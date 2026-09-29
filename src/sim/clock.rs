use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::astro::{
    epoch::EphemerisTime,
    units::{SECONDS_PER_DAY, SECONDS_PER_HOUR},
};

pub struct Clock {
    now: EphemerisTime,
    paused: bool,
    /// Either the next event, or None
    run_until: Option<EphemerisTime>,

    speed_idx: usize,

    pub can_speed_up: Rc<Cell<bool>>,
    pub can_slow_down: Rc<Cell<bool>>,
    pub sim_speed_str: Rc<RefCell<String>>,
}

impl Clock {
    const RATES: [(f64, &'static str); 6] = [
        (SECONDS_PER_HOUR, "1 hr/s"),
        (6.0 * SECONDS_PER_HOUR, "6 hrs/s"),
        (SECONDS_PER_DAY, "1 day/s"),
        (3.0 * SECONDS_PER_DAY, "3 days/s"),
        (7.0 * SECONDS_PER_DAY, "1 wk/s"),
        (30.0 * SECONDS_PER_DAY, "1 mo/s"),
    ];

    pub fn new() -> Self {
        let starting_idx = 1;
        Self {
            now: EphemerisTime::epoch(),
            paused: true,
            run_until: None,
            speed_idx: starting_idx,
            can_speed_up: Rc::new(Cell::new(true)),
            can_slow_down: Rc::new(Cell::new(true)),
            sim_speed_str: Rc::new(RefCell::new(String::from(Self::RATES[starting_idx].1))),
        }
    }

    pub fn get_rate(&self) -> f64 {
        Self::RATES[self.speed_idx].0
    }

    fn get_name(&self) -> &'static str {
        Self::RATES[self.speed_idx].1
    }

    pub fn speed_up(&mut self) {
        self.speed_idx = (self.speed_idx + 1).min(Self::RATES.len() - 1);
        self.can_slow_down.set(true);
        self.can_speed_up
            .set(self.speed_idx < Self::RATES.len() - 1);
        *self.sim_speed_str.borrow_mut() = String::from(self.get_name())
    }

    pub fn slow_down(&mut self) {
        self.speed_idx = self.speed_idx.saturating_sub(1);
        self.can_speed_up.set(true);
        self.can_slow_down.set(self.speed_idx > 0);
        *self.sim_speed_str.borrow_mut() = String::from(self.get_name())
    }
}

// TODO: Incorporate with Sim, next_stop(&Sim) combining the next event, resevoir limit and job completion
