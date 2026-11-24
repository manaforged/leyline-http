use std::time::Duration;

use crate::util::date::{month, unix_seconds};

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
        if (2..=3).contains(&parts.len())
            && let (Ok(hour), Ok(minute)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>())
        {
            let second = match parts.get(2) {
                Some(s) => match s.parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => return,
                },
                None => 0,
            };
            d.hour = hour;
            d.minute = minute;
            d.second = second;
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

    let secs = unix_seconds(
        i32::try_from(d.year).ok()?,
        d.month,
        d.day,
        (d.hour, d.minute, d.second),
    )?;
    Some(Duration::from_secs(u64::try_from(secs).unwrap_or(0)))
}
