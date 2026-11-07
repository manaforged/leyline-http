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

pub(crate) fn is_body_header(name: &str) -> bool {
    name.get(..CONTENT_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(CONTENT_PREFIX))
        || name.eq_ignore_ascii_case("transfer-encoding")
}

const CONTENT_PREFIX: &str = "content-";

pub(crate) fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub(crate) fn redirect_target(
    status: u16,
    headers: &http::HeaderMap,
) -> Option<std::borrow::Cow<'_, str>> {
    if !is_redirect_status(status) {
        return None;
    }
    headers
        .get(http::header::LOCATION)
        .map(|value| String::from_utf8_lossy(value.as_bytes()))
}

#[cfg(feature = "html")]
pub(crate) fn redirect_location(
    status: u16,
    headers: &http::HeaderMap,
    base: &url::Url,
) -> Option<url::Url> {
    redirect_target(status, headers).and_then(|to| base.join(&to).ok())
}

pub(crate) fn request_target(url: &url::Url) -> &str {
    &url[url::Position::BeforePath..url::Position::AfterQuery]
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

const SECRET_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

const CROSS_ORIGIN_EXTRA_HEADERS: [&str; 1] = ["host"];

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
    SECRET_HEADERS.iter().any(|s| name.eq_ignore_ascii_case(s))
}

pub(crate) fn masked<'a>(
    fields: impl Iterator<Item = (&'a str, &'a [u8])>,
) -> Vec<(&'a str, std::borrow::Cow<'a, str>)> {
    fields
        .map(|(name, value)| {
            let shown = if sensitive_header(name) {
                std::borrow::Cow::Borrowed("***")
            } else {
                String::from_utf8_lossy(value)
            };
            (name, shown)
        })
        .collect()
}

pub(crate) fn cross_origin_header(name: &str) -> bool {
    sensitive_header(name)
        || CROSS_ORIGIN_EXTRA_HEADERS
            .iter()
            .any(|s| name.eq_ignore_ascii_case(s))
}

pub(crate) fn basic_credentials(username: &str, password: &str) -> String {
    format!("Basic {}", base64_encode(&format!("{username}:{password}")))
}

pub(crate) fn proxy_basic_auth(proxy: &url::Url) -> Option<String> {
    if proxy.username().is_empty() && proxy.password().is_none() {
        return None;
    }
    let username = percent_decode(proxy.username());
    let password = proxy.password().map(percent_decode).unwrap_or_default();
    Some(basic_credentials(&username, &password))
}

pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub(crate) fn unquote(src: &str) -> (String, usize) {
    let mut out = String::new();
    let mut chars = src.char_indices().skip(1);
    while let Some((offset, c)) = chars.next() {
        match c {
            '"' => return (out, offset + 1),
            '\\' => out.extend(chars.next().map(|(_, escaped)| escaped)),
            other => out.push(other),
        }
    }
    (out, src.len())
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

pub(crate) mod coalesce;
pub(crate) mod date;
pub(crate) mod expiring;
pub(crate) mod upload;
pub(crate) mod version;
pub(crate) mod wait;

#[cfg(test)]
mod tests;

/// Calls `check` with every concatenation of up to `max_len` tokens from `alphabet`.
#[cfg(test)]
pub(crate) fn every_input(alphabet: &[&[u8]], max_len: usize, mut check: impl FnMut(&[u8])) {
    let mut digits: Vec<usize> = Vec::with_capacity(max_len);
    let mut input = Vec::new();
    loop {
        input.clear();
        for &digit in &digits {
            input.extend_from_slice(alphabet[digit]);
        }
        check(&input);
        if !next_input(&mut digits, alphabet.len(), max_len) {
            return;
        }
    }
}

#[cfg(test)]
fn next_input(digits: &mut Vec<usize>, base: usize, max_len: usize) -> bool {
    for digit in digits.iter_mut() {
        *digit += 1;
        if *digit < base {
            return true;
        }
        *digit = 0;
    }
    if digits.len() == max_len {
        return false;
    }
    digits.push(0);
    true
}

pub(crate) fn lookup<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key.trim()))
        .map(|(_, value)| *value)
}

pub(crate) fn product_token(agent: &str) -> &str {
    let agent = agent.trim();
    let end = agent
        .find(|c: char| !(c.is_ascii_alphabetic() || matches!(c, '_' | '-')))
        .unwrap_or(agent.len());
    &agent[..end]
}
