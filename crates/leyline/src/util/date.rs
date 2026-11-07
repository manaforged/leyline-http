const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

pub(crate) fn month(name: &str) -> Option<u32> {
    let prefix = name.get(..3)?;
    let index = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(prefix))?;
    u32::try_from(index + 1).ok()
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
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

pub(crate) fn unix_seconds(
    year: i32,
    month: u32,
    day: u32,
    (hour, minute, second): (u32, u32, u32),
) -> Option<i64> {
    days_from_civil(year, month, day)?
        .checked_mul(86400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))
}
