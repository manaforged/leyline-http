use crate::net::normalize_host;
use crate::util::domain_match;

pub(super) fn pattern_matches(host: &str, raw: &str) -> bool {
    let mut pat = raw.trim().to_string();
    if pat.is_empty() {
        return false;
    }
    if pat == "*" {
        return true;
    }
    if pat.starts_with('[') {
        if let Some(end) = pat.find("]:") {
            let suffix = &pat[end + 2..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                pat.truncate(end + 1);
            }
        }
    } else if pat.matches(':').count() == 1
        && let Some(idx) = pat.rfind(':')
        && pat[idx + 1..].bytes().all(|b| b.is_ascii_digit())
    {
        pat.truncate(idx);
    }
    let pat = normalize_host(&pat);
    domain_match(host, pat.strip_prefix('.').unwrap_or(&pat))
}
