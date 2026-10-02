use std::time::{Duration, SystemTime};

use crate::util::epoch_plus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WaitFormat {
    Seconds,
    UnixSeconds,
    HttpDate,
}

#[derive(Debug, Clone)]
pub(crate) struct WaitHeader {
    pub(crate) name: String,
    pub(crate) format: WaitFormat,
}

pub(crate) const RETRY_AFTER: &str = "retry-after";
const RETRY_AFTER_FORMATS: [WaitFormat; 2] = [WaitFormat::Seconds, WaitFormat::HttpDate];

impl WaitFormat {
    pub(crate) fn parse(self, value: &str) -> Option<Duration> {
        let value = value.trim();
        match self {
            Self::Seconds => parse_seconds(value),
            Self::UnixSeconds => until(epoch_plus(parse_seconds(value)?)?),
            Self::HttpDate => until(parse_imf_fixdate(value)?),
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
    let month = month_num(parts.next()?)?;
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

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const IMF_FIXDATE_YEARS: std::ops::RangeInclusive<i32> = 1970..=9999;

fn month_num(month: &str) -> Option<u32> {
    let index = MONTHS.iter().position(|m| *m == month)?;
    u32::try_from(index + 1).ok()
}

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
    let secs = days_from_civil(year, month, day)?
        .checked_mul(86400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec))?;
    epoch_plus(Duration::from_secs(u64::try_from(secs).ok()?))
}

fn date_in_range(year: i32, month: u32, day: u32) -> bool {
    IMF_FIXDATE_YEARS.contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = y.div_euclid(400);
    let yoe = u32::try_from(y.checked_sub(era.checked_mul(400)?)?).ok()?;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146097 + i64::from(doe) - 719468)
}
