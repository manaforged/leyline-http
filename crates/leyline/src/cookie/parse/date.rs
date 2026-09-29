use std::time::Duration;

use super::days_since_epoch;

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

    let m = month(tok);
    if m > 0 {
        d.month = m;
    }
}

fn month(tok: &str) -> u32 {
    match tok.get(..3).map(|s| s.to_lowercase()).as_deref() {
        Some("jan") => 1,
        Some("feb") => 2,
        Some("mar") => 3,
        Some("apr") => 4,
        Some("may") => 5,
        Some("jun") => 6,
        Some("jul") => 7,
        Some("aug") => 8,
        Some("sep") => 9,
        Some("oct") => 10,
        Some("nov") => 11,
        Some("dec") => 12,
        _ => 0,
    }
}

pub(super) fn stamp(d: &Date) -> Option<Duration> {
    if d.day == 0 || d.month == 0 || d.year == 0 {
        return None;
    }

    let days = days_since_epoch(d.year, d.month, d.day)?;
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
