#![forbid(unsafe_code)]
/// Returns the URL with its credentials and query masked as `***` and its fragment removed.
pub fn redact(raw: &str) -> String {
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

pub(crate) fn mask_query(text: &str) -> String {
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
    if url
        .set_username("")
        .and_then(|()| url.set_password(None))
        .is_err()
    {
        tracing::trace!("url cannot carry userinfo");
    }
    url
}

#[inline]
pub(crate) fn host_port(url: &url::Url) -> crate::core::Result<(&str, u16)> {
    match (url.host_str(), url.port_or_known_default()) {
        (Some(host), Some(port)) => Ok((host, port)),
        _ => Err(no_host_port(url)),
    }
}

#[cold]
fn no_host_port(url: &url::Url) -> crate::Error {
    if url.host_str().is_none() {
        return crate::Error::config("no host in URL");
    }
    crate::Error::config(format!("no default port for scheme {}", url.scheme()))
}

#[cfg(feature = "websocket")]
pub(crate) fn header_values<'a, K: AsRef<str>, V: AsRef<[u8]>>(
    headers: &'a [(K, V)],
    name: &'a str,
) -> impl Iterator<Item = &'a str> {
    headers
        .iter()
        .filter(move |(k, _)| k.as_ref().eq_ignore_ascii_case(name))
        .filter_map(|(_, v)| std::str::from_utf8(v.as_ref()).ok())
}

pub(crate) fn header_first<'a, K: AsRef<str>, V: AsRef<[u8]>>(
    headers: &'a [(K, V)],
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.as_ref().eq_ignore_ascii_case(name))
        .and_then(|(_, v)| std::str::from_utf8(v.as_ref()).ok())
}

pub(crate) fn content_length(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    let blank = |b: &u8| matches!(b, b' ' | b'\t');
    let start = bytes.iter().position(|b| !blank(b))?;
    let end = bytes.iter().rposition(|b| !blank(b))?;
    bytes[start..=end].iter().try_fold(0u64, |n, &b| {
        let digit = b.checked_sub(b'0').filter(|d| *d < 10)?;
        n.checked_mul(10)?.checked_add(u64::from(digit))
    })
}

pub(crate) fn header_contains_token<K: AsRef<str>, V: AsRef<[u8]>>(
    headers: &[(K, V)],
    name: &str,
    token: &str,
) -> bool {
    header_first(headers, name).is_some_and(|v| has_token(v, token))
}

pub(crate) fn has_token(value: &str, token: &str) -> bool {
    value
        .split(',')
        .any(|part| part.trim().eq_ignore_ascii_case(token))
}

pub(crate) fn request_target(url: &url::Url) -> &str {
    &url[url::Position::BeforePath..url::Position::AfterQuery]
}

pub(crate) fn authority<'a>(
    url: &'a url::Url,
    host: &'a str,
    port: u16,
) -> std::borrow::Cow<'a, str> {
    let is_default_port =
        (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
    if is_default_port {
        std::borrow::Cow::Borrowed(host)
    } else if url.port() == Some(port) && url.host_str() == Some(host) {
        std::borrow::Cow::Borrowed(&url[url::Position::BeforeHost..url::Position::AfterPort])
    } else {
        std::borrow::Cow::Owned(format!("{host}:{port}"))
    }
}

pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn read<T>(rw: &std::sync::RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    rw.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn write<T>(rw: &std::sync::RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    rw.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const FAR_FUTURE: std::time::Duration = std::time::Duration::from_secs(100 * 365 * 24 * 3600);

pub(crate) fn saturating_after(
    now: std::time::Instant,
    after: std::time::Duration,
) -> std::time::Instant {
    now.checked_add(after)
        .or_else(|| now.checked_add(FAR_FUTURE.min(after)))
        .unwrap_or(now)
}

pub(crate) fn epoch_plus(after: std::time::Duration) -> Option<std::time::SystemTime> {
    std::time::SystemTime::UNIX_EPOCH.checked_add(after)
}

pub(crate) fn since_epoch(at: std::time::SystemTime) -> std::time::Duration {
    at.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

const SENSITIVE_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

pub(crate) fn has_ascii_uppercase(text: &str) -> bool {
    const ASCII: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const ONES: u64 = 0x0101_0101_0101_0101;
    const FROM_A: u64 = ONES * (0x80 - b'A' as u64);
    const PAST_Z: u64 = ONES * (0x80 - b'Z' as u64 - 1);
    let bytes = text.as_bytes();
    let (Some(tail), (words, _)) = (bytes.last_chunk::<8>(), bytes.as_chunks::<8>()) else {
        return bytes.iter().any(u8::is_ascii_uppercase);
    };
    words.iter().chain([tail]).any(|word| {
        let word = u64::from_ne_bytes(*word);
        let ascii = word & ASCII;
        (ascii + FROM_A) & !(ascii + PAST_Z) & !word & !ASCII != 0
    })
}

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

pub(crate) fn random_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut out = vec![0u8; len];
    rand::rng().fill_bytes(&mut out);
    out
}

pub(crate) fn random_hex_token(bytes: usize) -> String {
    hex::encode(random_bytes(bytes))
}

pub(crate) fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

pub(crate) fn delta_seconds(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(value.parse().unwrap_or(u64::MAX))
}

pub(crate) fn is_idempotent(method: &str) -> bool {
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

pub(crate) const MAX_INFORMATIONAL: usize = 16;

pub(crate) mod atomic;
pub(crate) mod autosave;
pub(crate) mod date;
pub(crate) mod upload;

#[cfg(test)]
mod tests;
