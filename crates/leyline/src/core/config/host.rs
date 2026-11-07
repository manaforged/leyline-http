pub(crate) fn normalize_host(host: &str) -> String {
    let stripped = host.trim().trim_end_matches('.');
    let stripped = stripped
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stripped);
    let lowered = stripped.to_ascii_lowercase();
    match url::Host::parse(&lowered) {
        Ok(url::Host::Domain(d)) => d,
        Ok(url::Host::Ipv4(a)) => a.to_string(),
        Ok(url::Host::Ipv6(a)) => a.to_string(),
        Err(_) => lowered,
    }
}

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

pub(crate) fn domain_match(host: &str, domain: &str) -> bool {
    let Some(rest) = host.len().checked_sub(domain.len()) else {
        return false;
    };
    host.as_bytes()[rest..].eq_ignore_ascii_case(domain.as_bytes())
        && (rest == 0 || host.as_bytes()[rest - 1] == b'.')
}

pub(crate) fn registrable_domain(host: &str) -> Option<String> {
    psl::domain(host.as_bytes()).map(|d| String::from_utf8_lossy(d.as_bytes()).into_owned())
}
