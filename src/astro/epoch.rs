use std::ops::{Add, AddAssign, Div, Mul, Sub};

use chrono::{DateTime, Datelike, Timelike, Utc};

use crate::astro::units::{
    SECONDS_PER_DAY, SECONDS_PER_HOUR, SECONDS_PER_MINUTE, SECONDS_PER_YEAR,
};

/// Represents a duration in microseconds. Should allow for ~292,000 years future and past.
///
/// When used as a time point, duration since the save-start epoch.
/// TODO: Separate Instant and Duration types
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Ord, Eq)]
pub struct EphemerisTime(i64);

pub const ET_PER_SECOND: f64 = 1_000_000.0;
const ET_PER_YEAR: f64 = SECONDS_PER_YEAR * ET_PER_SECOND;
const ET_PER_DAY: f64 = SECONDS_PER_DAY * ET_PER_SECOND;
const ET_PER_MINUTE: f64 = SECONDS_PER_MINUTE * ET_PER_SECOND;
const ET_PER_HOUR: f64 = SECONDS_PER_HOUR * ET_PER_SECOND;

impl EphemerisTime {
    const EPOCH_UNIX_SECS: i64 = -62_167_219_200;

    pub const fn new(microsecs: i64) -> Self {
        Self(microsecs)
    }

    pub const fn from_years(years: f64) -> Self {
        Self((years * ET_PER_YEAR) as i64)
    }

    pub const fn from_days(days: f64) -> Self {
        Self((days * ET_PER_DAY) as i64)
    }

    pub const fn from_mins(mins: f64) -> Self {
        Self((mins * ET_PER_MINUTE) as i64)
    }

    pub const fn from_secs(secs: f64) -> Self {
        Self((secs * ET_PER_SECOND) as i64)
    }

    pub const fn as_years(self) -> f64 {
        (self.0 as f64) / ET_PER_YEAR
    }

    pub const fn as_days(self) -> f64 {
        (self.0 as f64) / ET_PER_DAY
    }

    pub const fn as_hours(self) -> f64 {
        (self.0 as f64) / ET_PER_HOUR
    }

    pub const fn as_secs(self) -> f64 {
        (self.0 as f64) / ET_PER_SECOND
    }

    pub fn ceil_to(self, step: Self) -> Self {
        let (t, s) = (self.0, step.0.max(1));
        Self(((t + s - 1).div_euclid(s)) * s)
    }

    const fn as_datetime(&self) -> Option<DateTime<Utc>> {
        let secs = self.0.div_euclid(1_000_000);
        let micros = self.0.rem_euclid(1_000_000) * 1000; // always positive

        chrono::DateTime::from_timestamp(secs, micros as u32)
    }

    pub const fn epoch() -> Self {
        Self(Self::EPOCH_UNIX_SECS * 1_000_000)
    }

    pub fn as_calendar(&self) -> Option<String> {
        let dt = self.as_datetime()?;
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            dt.year(),
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute()
        ))
    }

    pub fn short_month_name(&self) -> Option<&'static str> {
        let dt = self.as_datetime()?;
        Some(
            [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ][dt.month() as usize - 1],
        )
    }

    pub fn day_of_month(&self) -> Option<String> {
        let dt = self.as_datetime()?;
        Some(format!("{:02}", dt.day()))
    }

    pub fn year(&self) -> Option<i32> {
        Some(self.as_datetime()?.year())
    }

    pub fn hour_minute(&self) -> Option<String> {
        let dt = self.as_datetime()?;
        Some(format!("{:02}:{:02}", dt.hour(), dt.minute()))
    }

    pub fn short_datetime(&self) -> Option<String> {
        let dt = self.as_datetime()?;
        Some(format!(
            "{:02} {} {:02}:{:02}",
            dt.day(),
            self.short_month_name()?,
            dt.hour(),
            dt.minute()
        ))
    }

    pub fn short_date(&self) -> Option<String> {
        let dt = self.as_datetime()?;
        Some(format!(
            "{:02} {} {:04}",
            dt.day(),
            self.short_month_name()?,
            dt.year(),
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_constant_matches_chrono() {
        let expected = chrono::NaiveDate::from_ymd_opt(0, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(EphemerisTime::EPOCH_UNIX_SECS, expected);
        assert_eq!(
            EphemerisTime::epoch().as_calendar().as_deref(),
            Some("0000-01-01 00:00")
        );
    }

    #[test]
    fn unit_conversions_round_trip() {
        assert_eq!(EphemerisTime::from_years(1.0).as_years(), 1.0);
        assert_eq!(EphemerisTime::from_days(0.5).as_days(), 0.5);
        assert_eq!(EphemerisTime::from_mins(90.0).as_hours(), 1.5);
        assert_eq!(EphemerisTime::from_secs(1.5), EphemerisTime::new(1_500_000));
        assert_eq!(
            EphemerisTime::from_days(1.0),
            EphemerisTime::from_secs(86_400.0)
        );
        assert_eq!(
            EphemerisTime::from_years(1.0),
            EphemerisTime::new(31_536_000_000_000)
        ); // 365 days
        assert_eq!(
            EphemerisTime::from_mins(1.0),
            EphemerisTime::new(60_000_000)
        );
    }

    #[test]
    fn ceil_to_rounds_up_to_the_next_step() {
        let step = EphemerisTime::new(5);
        assert_eq!(EphemerisTime::new(10).ceil_to(step), EphemerisTime::new(10)); // already on a step
        assert_eq!(EphemerisTime::new(11).ceil_to(step), EphemerisTime::new(15));
        assert_eq!(EphemerisTime::new(-7).ceil_to(step), EphemerisTime::new(-5)); // up, not toward zero
        assert_eq!(EphemerisTime::new(-5).ceil_to(step), EphemerisTime::new(-5));
        // a zero step is treated as 1 microsecond rather than dividing by zero
        assert_eq!(
            EphemerisTime::new(3).ceil_to(EphemerisTime::new(0)),
            EphemerisTime::new(3)
        );
    }

    #[test]
    fn calendar_formatting() {
        let unix_origin = EphemerisTime::new(0);
        assert_eq!(
            unix_origin.as_calendar().as_deref(),
            Some("1970-01-01 00:00")
        );
        assert_eq!(unix_origin.short_date().as_deref(), Some("01 Jan 1970"));
        assert_eq!(
            unix_origin.short_datetime().as_deref(),
            Some("01 Jan 00:00")
        );
        assert_eq!(unix_origin.year(), Some(1970));

        let new_years_eve = unix_origin - EphemerisTime::from_days(1.0);
        assert_eq!(new_years_eve.short_month_name(), Some("Dec"));
        assert_eq!(new_years_eve.day_of_month().as_deref(), Some("31"));
    }

    #[test]
    fn times_before_the_unix_origin_format_correctly() {
        // one microsecond before 1970: the sub-second part must not go negative
        let t = EphemerisTime::new(-1);
        assert_eq!(t.as_calendar().as_deref(), Some("1969-12-31 23:59"));
        assert_eq!(t.hour_minute().as_deref(), Some("23:59"));
    }

    #[test]
    fn times_outside_chronos_range_dont_format() {
        // i64 microseconds reach ±292,000 years, chrono only ±262,000
        assert_eq!(EphemerisTime::new(i64::MAX).as_calendar(), None);
        assert_eq!(EphemerisTime::new(i64::MIN).year(), None);
    }

    #[test]
    fn short_duration_picks_the_two_largest_units() {
        assert_eq!(EphemerisTime::from_mins(5.0).short_duration(), "0h 05m");
        assert_eq!(
            EphemerisTime::from_secs(3.0 * 3600.0 + 7.0 * 60.0).short_duration(),
            "3h 07m"
        );
        assert_eq!(
            (EphemerisTime::from_days(2.0) + EphemerisTime::from_secs(5.0 * 3600.0))
                .short_duration(),
            "2d 05h"
        );
        assert_eq!(EphemerisTime::from_days(400.0).short_duration(), "1y 35d"); // 365-day years
        assert_eq!(EphemerisTime::from_secs(-100.0).short_duration(), "0h 00m");
        // clamps to zero
    }

    #[test]
    fn arithmetic_and_ordering() {
        let a = EphemerisTime::from_secs(10.0);
        let b = EphemerisTime::from_secs(4.0);
        assert_eq!(a + b, EphemerisTime::from_secs(14.0));
        assert_eq!(a - b, EphemerisTime::from_secs(6.0));
        assert_eq!(b * 3, EphemerisTime::from_secs(12.0));
        assert_eq!(a / 2, EphemerisTime::from_secs(5.0));

        let mut c = a;
        c += b;
        assert_eq!(c, a + b);

        assert!(b < a);
    }

    #[test]
    fn sub_second_part_survives_conversion() {
        let t = EphemerisTime::new(1_500_000); // 1.5 s after the Unix origin
        assert_eq!(
            t.as_datetime().unwrap().timestamp_subsec_nanos(),
            500_000_000
        );
    }
}
