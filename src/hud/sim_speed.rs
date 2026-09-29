use crate::astro::units::{SECONDS_PER_DAY, SECONDS_PER_HOUR};

pub struct SimSpeed {
    idx: usize,
}

impl SimSpeed {
    const RATES: [(f64, &'static str); 6] = [
        (SECONDS_PER_HOUR, "1 hr/s"),
        (6.0 * SECONDS_PER_HOUR, "6 hrs/s"),
        (SECONDS_PER_DAY, "1 day/s"),
        (3.0 * SECONDS_PER_DAY, "3 days/s"),
        (7.0 * SECONDS_PER_DAY, "1 wk/s"),
        (30.0 * SECONDS_PER_DAY, "1 mo/s"),
    ];

    pub fn new() -> Self {
        Self { idx: 1 }
    }

    pub fn get_rate(&self) -> f64 {
        Self::RATES[self.idx].0
    }

    pub fn rate_label(&self) -> &'static str {
        Self::RATES[self.idx].1
    }

    pub fn can_speed_up(&self) -> bool {
        self.idx < Self::RATES.len() - 1
    }

    pub fn can_slow_down(&self) -> bool {
        self.idx > 0
    }

    pub fn speed_up(&mut self) {
        self.idx = (self.idx + 1).min(Self::RATES.len() - 1);
    }

    pub fn slow_down(&mut self) {
        self.idx = self.idx.saturating_sub(1);
    }
}
