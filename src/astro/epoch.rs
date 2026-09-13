use std::{
    fmt::format,
    ops::{Add, AddAssign, Div, Mul, Sub},
};

use chrono::{DateTime, Datelike, Timelike, Utc};

use crate::astro::units::{SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_YEAR};

/// Represents a duration in microseconds. Should allow for ~292,000 years future and past.
///
/// When used as a time point, duration since the save-start epoch.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
pub struct EphemerisTime(i64);

pub const ET_PER_SECOND: f64 = 1_000_000.0;
#[allow(dead_code)]
const ET_PER_DAY: f64 = SECONDS_PER_DAY * ET_PER_SECOND;
const ET_PER_YEAR: f64 = SECONDS_PER_YEAR * ET_PER_SECOND;
const ET_PER_HOUR: f64 = SECONDS_PER_HOUR * ET_PER_SECOND;

impl EphemerisTime {
    pub fn new(microsecs: i64) -> Self {
        Self(microsecs)
    }

    pub fn from_years(years: f64) -> Self {
        Self((years * ET_PER_YEAR) as i64)
    }

    #[allow(dead_code)]
    pub fn from_days(days: f64) -> Self {
        Self((days * ET_PER_DAY) as i64)
    }

    pub fn from_secs(secs: f64) -> Self {
        Self((secs * ET_PER_SECOND) as i64)
    }

    pub fn as_years(self) -> f64 {
        (self.0 as f64) / ET_PER_YEAR
    }

    #[allow(dead_code)]
    pub fn as_days(self) -> f64 {
        (self.0 as f64) / ET_PER_DAY
    }

    #[allow(dead_code)]
    pub fn as_hours(self) -> f64 {
        (self.0 as f64) / ET_PER_HOUR
    }

    pub fn as_secs(self) -> f64 {
        (self.0 as f64) / ET_PER_SECOND
    }

    pub fn ceil_to(self, step: Self) -> Self {
        let (t, s) = (self.0, step.0.max(1));
        Self(((t + s - 1).div_euclid(s)) * s)
    }

    fn as_datetime(&self) -> DateTime<Utc> {
        let secs = self.0.div_euclid(1_000_000);
        let micros = self.0.rem_euclid(1_000_000) * 1000; // always positive

        chrono::DateTime::from_timestamp(secs, micros as u32).unwrap()
    }

    pub fn as_calendar(&self) -> String {
        let dt = self.as_datetime();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second()
        )
    }

    pub fn short_month_name(&self) -> String {
        let dt = self.as_datetime();
        [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ][dt.month() as usize - 1]
            .to_string()
    }

    pub fn day_of_month(&self) -> String {
        let dt = self.as_datetime();
        format!("{:02}", dt.day())
    }

    pub fn short_datetime(&self) -> String {
        let dt = self.as_datetime();
        format!(
            "{:02} {} {:02}:{:02}",
            dt.day(),
            self.short_month_name(),
            dt.hour(),
            dt.minute()
        )
    }

    pub fn short_date(&self) -> String {
        let dt = self.as_datetime();
        format!(
            "{:02} {} {:04}",
            dt.day(),
            self.short_month_name(),
            dt.year(),
        )
    }

    /// Compact duration formatting. Negative durations clamp to zero.
    pub fn short_duration(&self) -> String {
        let secs = self.as_secs().max(0.0) as i64;

        let (year, day, hour) = (
            SECONDS_PER_YEAR as i64,
            SECONDS_PER_DAY as i64,
            SECONDS_PER_HOUR as i64,
        );

        let (y, d) = (secs / year, secs % year / day);
        let (h, m) = (secs % day / hour, secs % hour / 60);

        if y > 0 {
            format!("{y}y {d}d")
        } else if d > 0 {
            format!("{d}d {h:02}h")
        } else {
            format!("{h}h {m:02}m")
        }
    }

    pub fn epoch() -> Self {
        let dt = chrono::NaiveDate::from_ymd_opt(0, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        Self(dt.and_utc().timestamp() * 1_000_000)
    }
}

impl Add for EphemerisTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for EphemerisTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul<i64> for EphemerisTime {
    type Output = Self;
    fn mul(self, rhs: i64) -> Self {
        Self(self.0 * rhs)
    }
}

impl Div<i64> for EphemerisTime {
    type Output = Self;
    fn div(self, rhs: i64) -> Self {
        Self(self.0 / rhs)
    }
}

impl AddAssign for EphemerisTime {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}
