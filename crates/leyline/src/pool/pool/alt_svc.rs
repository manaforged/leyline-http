use std::collections::HashMap;
use std::time::{Duration, SystemTime};

const DEFAULT_MAX_AGE: Duration = Duration::from_secs(86_400);

const MAX_ORIGINS: usize = 1024;

#[derive(Default)]
pub(crate) struct AltSvcCache {
    h3: HashMap<(String, u16), SystemTime>,
}

impl AltSvcCache {
    pub(crate) fn note(
        &mut self,
        host: &str,
        port: u16,
        fields: &[&str],
        age: Duration,
        now: SystemTime,
    ) {
        let key = (host.to_string(), port);
        let expiry = h3_max_age(fields, host, port)
            .and_then(|max_age| max_age.checked_sub(age))
            .filter(|fresh| !fresh.is_zero())
            .and_then(|fresh| now.checked_add(fresh));
        let Some(expiry) = expiry else {
            self.h3.remove(&key);
            return;
        };
        if !self.h3.contains_key(&key) && self.h3.len() >= MAX_ORIGINS {
            self.make_room(now);
        }
        self.h3.insert(key, expiry);
    }

    pub(crate) fn knows_h3(&mut self, host: &str, port: u16, now: SystemTime) -> bool {
        let key = (host.to_string(), port);
        match self.h3.get(&key) {
            Some(expiry) if *expiry > now => true,
            Some(_) => {
                self.h3.remove(&key);
                false
            }
            None => false,
        }
    }

    fn make_room(&mut self, now: SystemTime) {
        self.h3.retain(|_, expiry| *expiry > now);
        if self.h3.len() < MAX_ORIGINS {
            return;
        }
        if let Some(oldest) = self
            .h3
            .iter()
            .min_by_key(|(_, expiry)| **expiry)
            .map(|(key, _)| key.clone())
        {
            self.h3.remove(&oldest);
        }
    }
}

fn h3_max_age(fields: &[&str], host: &str, port: u16) -> Option<Duration> {
    let entries = || fields.iter().flat_map(|field| split_unquoted(field, ','));
    if entries().any(|alt| alt.trim().eq_ignore_ascii_case("clear")) {
        return None;
    }
    entries()
        .filter_map(|alt| same_authority_h3(alt.trim_start(), host, port))
        .max()
}

fn same_authority_h3(entry: &str, host: &str, port: u16) -> Option<Duration> {
    let mut parts = split_unquoted(entry.strip_prefix("h3=")?, ';');
    let authority = parts.next().unwrap_or_default().trim();
    let authority = authority
        .strip_prefix('"')
        .and_then(|a| a.strip_suffix('"'))
        .unwrap_or(authority);
    let (alt_host, alt_port) = authority.rsplit_once(':')?;
    if alt_port.parse::<u16>() != Ok(port)
        || !(alt_host.is_empty() || alt_host.eq_ignore_ascii_case(host))
    {
        return None;
    }
    let ma = parts
        .filter_map(|param| param.trim().split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("ma"));
    match ma {
        None => Some(DEFAULT_MAX_AGE),
        Some((_, secs)) => crate::util::delta_seconds(secs.trim().trim_matches('"'))
            .map(|secs| Duration::from_secs(secs.min(u64::from(u32::MAX)))),
    }
}

fn split_unquoted(value: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    let mut bounds = Vec::new();
    for (index, ch) in value.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if ch == separator && !quoted => {
                bounds.push((start, index));
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    bounds.push((start, value.len()));
    bounds.into_iter().map(move |(from, to)| &value[from..to])
}
