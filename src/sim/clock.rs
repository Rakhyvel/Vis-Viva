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
        Self {
            now: EphemerisTime::epoch(),
            paused: true,
            run_until: None,
            speed_idx: 1,
        }
    }

    pub fn get_rate(&self) -> f64 {
        Self::RATES[self.speed_idx].0
    }

    pub fn rate_label(&self) -> &'static str {
        Self::RATES[self.speed_idx].1
    }

    pub fn can_speed_up(&self) -> bool {
        self.speed_idx < Self::RATES.len() - 1
    }

    pub fn can_slow_down(&self) -> bool {
        self.speed_idx > 0
    }

    pub fn speed_up(&mut self) {
        self.speed_idx = (self.speed_idx + 1).min(Self::RATES.len() - 1);
    }

    pub fn slow_down(&mut self) {
        self.speed_idx = self.speed_idx.saturating_sub(1);
    }

    pub fn now(&self) -> EphemerisTime {
        self.now
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_run_until(&mut self, run_until: Option<EphemerisTime>) {
        self.run_until = run_until;
    }

    /// Advance by one frame at the current rate. Returns Some(t) if we hit run_until and paused there
    pub fn advance(&mut self, real_dt: f64) -> Option<EphemerisTime> {
        let mut t = self.now + EphemerisTime::from_secs(real_dt * self.get_rate());
        let stopped = self.run_until.is_some_and(|stop| t >= stop);
        if stopped {
            t = self.run_until.unwrap();
            self.paused = true;
        }
        self.now = t;
        stopped.then_some(t)
    }

    /// Pause and forget the next stop
    pub fn stop(&mut self) {
        self.paused = true;
        self.run_until = None
    }
}
