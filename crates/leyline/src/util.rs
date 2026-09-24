#![forbid(unsafe_code)]
pub(crate) fn redact(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    let masked = if parsed.password().is_some() {
        parsed.set_password(Some("***"))
    } else if !parsed.username().is_empty() {
        parsed.set_username("***")
    } else {
        return raw.to_string();
    };
    match masked {
        Ok(()) => parsed.to_string(),
        Err(()) => raw.to_string(),
    }
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
