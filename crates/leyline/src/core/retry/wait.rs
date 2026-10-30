use std::time::{Duration, SystemTime};

use crate::util::epoch_plus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WaitFormat {
    Seconds,
    UnixSeconds,
    HttpDate,
    Millis,
    Delay,
}

#[derive(Debug, Clone)]
pub(crate) struct WaitHeader {
    pub(crate) name: String,
    pub(crate) format: WaitFormat,
}

pub(crate) const RETRY_AFTER: &str = "retry-after";
const RETRY_AFTER_FORMATS: [WaitFormat; 2] = [WaitFormat::Seconds, WaitFormat::HttpDate];
const UNIX_SECONDS_FLOOR: u64 = 1_000_000_000;

impl WaitFormat {
    pub(crate) fn parse(self, value: &str) -> Option<Duration> {
        let value = value.trim();
        match self {
            Self::Seconds => parse_seconds(value),
            Self::UnixSeconds => until(epoch_plus(parse_seconds(value)?)?),
            Self::HttpDate => until(parse_imf_fixdate(value)?),
            Self::Millis => Some(Duration::from_millis(value.parse().ok()?)),
            Self::Delay => match parse_seconds(value) {
                Some(secs) if secs.as_secs() >= UNIX_SECONDS_FLOOR => {
                    Self::UnixSeconds.parse(value)
                }
                Some(secs) => Some(secs),
                None => Self::HttpDate.parse(value),
            },
        }
    }
}

pub(crate) fn parse_retry_after(value: &str) -> Option<Duration> {
    RETRY_AFTER_FORMATS
        .iter()
        .find_map(|format| format.parse(value))
}

fn parse_seconds(value: &str) -> Option<Duration> {
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let secs = value.parse::<f64>().ok()?;
    Duration::try_from_secs_f64(secs).ok()
}

fn until(when: SystemTime) -> Option<Duration> {
    Some(
        when.duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

fn parse_imf_fixdate(s: &str) -> Option<SystemTime> {
    let rest = s.split_once(", ")?.1;
    let mut parts = rest.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = crate::util::date::month(parts.next()?)?;
    let year: i32 = parts.next()?.parse().ok()?;
    let hms = parts.next()?;
    let tz = parts.next()?;
    if !tz.eq_ignore_ascii_case("GMT") && !tz.eq_ignore_ascii_case("UTC") {
        return None;
    }
    let mut t = hms.split(':');
    let hour: u32 = t.next()?.parse().ok()?;
    let min: u32 = t.next()?.parse().ok()?;
    let sec: u32 = t.next()?.parse().ok()?;
    unix_from_ymd_hms(year, month, day, hour, min, sec)
}

const IMF_FIXDATE_YEARS: std::ops::RangeInclusive<i32> = 1970..=9999;

pub(super) fn unix_from_ymd_hms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    min: u32,
    sec: u32,
) -> Option<SystemTime> {
    if !date_in_range(year, month, day) || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    let secs = crate::util::date::days_from_civil(year, month, day)?
        .checked_mul(86400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec))?;
    epoch_plus(Duration::from_secs(u64::try_from(secs).ok()?))
}

fn date_in_range(year: i32, month: u32, day: u32) -> bool {
    IMF_FIXDATE_YEARS.contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
}
