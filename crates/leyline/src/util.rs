#![forbid(unsafe_code)]
pub(crate) fn redact(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return mask_userinfo(&mask_query(raw));
    };
    let mut changed = if parsed.password().is_some() {
        parsed.set_password(Some("***")).is_ok()
    } else if !parsed.username().is_empty() {
        parsed.set_username("***").is_ok()
    } else {
        false
    };
    if parsed.query().is_some() || parsed.fragment().is_some() {
        if parsed.query().is_some() {
            parsed.set_query(Some("***"));
        }
        parsed.set_fragment(None);
        changed = true;
    }
    let shown = if changed {
        parsed.to_string()
    } else {
        raw.to_string()
    };
    if parsed.has_host() {
        shown
    } else {
        mask_userinfo(&shown)
    }
}

fn mask_query(text: &str) -> String {
    let text = text.split_once('#').map_or(text, |(head, _)| head);
    match text.split_once('?') {
        Some((head, _)) => format!("{head}?***"),
        None => text.to_owned(),
    }
}

fn mask_userinfo(text: &str) -> String {
    let Some((head, host)) = text.rsplit_once('@') else {
        return text.to_owned();
    };
    let (prefix, userinfo) = match head.split_once("://") {
        Some((scheme, _)) if is_scheme(scheme) => head.split_at(scheme.len() + 3),
        _ => ("", head),
    };
    let masked = match userinfo.split_once(':') {
        Some((user, _)) => format!("{user}:***"),
        None => "***".to_owned(),
    };
    format!("{prefix}{masked}@{host}")
}

fn is_scheme(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

pub(crate) fn without_userinfo(mut url: url::Url) -> url::Url {
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url
}

pub(crate) fn redact_target(target: &str) -> String {
    match target.split_once('?') {
        Some((path, _)) => format!("{path}?***"),
        None => target.to_owned(),
    }
}

pub(crate) fn request_target(url: &url::Url) -> &str {
    &url[url::Position::BeforePath..url::Position::AfterQuery]
}

pub(crate) fn authority(url: &url::Url, host: &str, port: u16) -> String {
    let is_default_port =
        (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
    if is_default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn epoch_plus(after: std::time::Duration) -> Option<std::time::SystemTime> {
    std::time::SystemTime::UNIX_EPOCH.checked_add(after)
}

const SENSITIVE_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

pub(crate) fn sensitive_header(name: &str) -> bool {
    SENSITIVE_HEADERS
        .iter()
        .any(|s| name.eq_ignore_ascii_case(s))
}

pub(crate) fn proxy_basic_auth(proxy: &url::Url) -> Option<String> {
    if proxy.username().is_empty() && proxy.password().is_none() {
        return None;
    }
    let username = percent_decode(proxy.username());
    let password = proxy.password().map(percent_decode).unwrap_or_default();
    Some(format!(
        "Basic {}",
        base64_encode(&format!("{username}:{password}"))
    ))
}

pub(crate) fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

#[cfg(feature = "http3")]
pub(crate) fn unspecified_for(peer: std::net::SocketAddr) -> std::net::SocketAddr {
    let ip: std::net::IpAddr = match peer {
        std::net::SocketAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
        std::net::SocketAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
    };
    std::net::SocketAddr::new(ip, 0)
}

pub(crate) fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

pub(crate) fn random_hex_token(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

pub(crate) fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const DELTA_SECONDS_OVERFLOW: u64 = 1 << 31;

pub(crate) fn delta_seconds(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(value.parse().unwrap_or(DELTA_SECONDS_OVERFLOW))
}

pub(crate) fn is_idempotent(method: &str) -> bool {
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
