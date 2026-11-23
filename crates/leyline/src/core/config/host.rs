use crate::net::normalize_host;
use crate::util::domain_match;

pub(super) fn pattern_matches_port(host: &str, port: Option<u16>, raw: &str) -> bool {
    let mut pat = raw.trim().to_string();
    if pat.is_empty() {
        return false;
    }
    if pat == "*" {
        return true;
    }
    let mut pat_port = None;
    if pat.starts_with('[') {
        if let Some(end) = pat.find("]:") {
            let suffix = &pat[end + 2..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                let Ok(parsed) = suffix.parse::<u16>() else {
                    return false;
                };
                pat_port = Some(parsed);
                pat.truncate(end + 1);
            }
        }
    } else if pat.matches(':').count() == 1
        && let Some(idx) = pat.rfind(':')
        && !pat[idx + 1..].is_empty()
        && pat[idx + 1..].bytes().all(|b| b.is_ascii_digit())
    {
        let Ok(parsed) = pat[idx + 1..].parse::<u16>() else {
            return false;
        };
        pat_port = Some(parsed);
        pat.truncate(idx);
    }
    if let (Some(want), Some(actual)) = (pat_port, port)
        && want != actual
    {
        return false;
    }
    let pat = normalize_host(&pat);
    domain_match(host, pat.strip_prefix('.').unwrap_or(&pat))
}
