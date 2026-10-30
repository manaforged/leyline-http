use std::time::Duration;

use crate::util::date::{days_from_civil, month};

const MIN_YEAR: u32 = 1601;

#[derive(Default)]
pub(super) struct Date {
    day: u32,
    month: u32,
    year: u32,
    hour: u32,
    minute: u32,
    second: u32,
    time: bool,
}

pub(super) fn token(d: &mut Date, tok: &str) {
    if !d.time && tok.contains(':') {
        let parts: Vec<&str> = tok.split(':').collect();
        if parts.len() >= 2 {
            d.hour = parts[0].parse().unwrap_or(0);
            d.minute = parts[1].parse().unwrap_or(0);
            d.second = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            d.time = true;
        }
        return;
    }

    if let Ok(n) = tok.parse::<u32>() {
        if n > 99 {
            d.year = n;
        } else if d.day == 0 {
            d.day = n;
        } else if d.year == 0 {
            d.year = if n > 68 { 1900 + n } else { 2000 + n };
        }
        return;
    }

    if let Some(m) = month(tok) {
        d.month = m;
    }
}

pub(super) fn stamp(d: &Date) -> Option<Duration> {
    if d.day == 0 || d.month == 0 || d.year < MIN_YEAR {
        return None;
    }
    if d.hour > 23 || d.minute > 59 || d.second > 59 {
        return None;
    }

    let days = days_from_civil(i32::try_from(d.year).ok()?, d.month, d.day)?;
    if days < 0 {
        return Some(Duration::ZERO);
    }
    let secs = (days as u64)
        .checked_mul(86400)?
        .checked_add(d.hour as u64 * 3600)?
        .checked_add(d.minute as u64 * 60)?
        .checked_add(d.second as u64)?;
    Some(Duration::from_secs(secs))
}
