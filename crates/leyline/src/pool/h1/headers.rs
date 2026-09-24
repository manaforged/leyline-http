use super::*;

pub(super) fn path_and_query(url: &url::Url) -> String {
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    match url.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_string(),
    }
}

pub(super) fn authority_for(url: &url::Url, host: &str, port: u16) -> String {
    let is_default_port =
        (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
    if is_default_port {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

pub(super) fn contains_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

pub(super) fn is_valid_token(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| {
        matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
                | b'0'..=b'9'
                | b'A'..=b'Z'
                | b'a'..=b'z'
        )
    })
}

pub(super) fn is_valid_header_value(s: &str) -> bool {
    s.bytes()
        .all(|b| matches!(b, b'\t' | b' '..=b'~' | 0x80..=0xFF))
}

pub(super) fn is_valid_request_target(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b > 0x20 && b != 0x7F)
}

pub(super) fn h1_header_name(name: &str) -> Cow<'_, str> {
    match name {
        _ if name.eq_ignore_ascii_case("host") => Cow::Borrowed("Host"),
        _ if name.eq_ignore_ascii_case("connection") => Cow::Borrowed("Connection"),
        _ if name.eq_ignore_ascii_case("user-agent") => Cow::Borrowed("User-Agent"),
        _ if name.eq_ignore_ascii_case("accept") => Cow::Borrowed("Accept"),
        _ if name.eq_ignore_ascii_case("accept-encoding") => Cow::Borrowed("Accept-Encoding"),
        _ if name.eq_ignore_ascii_case("accept-language") => Cow::Borrowed("Accept-Language"),
        _ if name.eq_ignore_ascii_case("content-length") => Cow::Borrowed("Content-Length"),
        _ if name.eq_ignore_ascii_case("content-type") => Cow::Borrowed("Content-Type"),
        _ if name.eq_ignore_ascii_case("cookie") => Cow::Borrowed("Cookie"),
        _ if name.eq_ignore_ascii_case("authorization") => Cow::Borrowed("Authorization"),
        _ if name.eq_ignore_ascii_case("proxy-authorization") => {
            Cow::Borrowed("Proxy-Authorization")
        }
        _ if name.eq_ignore_ascii_case("origin") => Cow::Borrowed("Origin"),
        _ if name.eq_ignore_ascii_case("referer") => Cow::Borrowed("Referer"),
        _ if name.eq_ignore_ascii_case("upgrade") => Cow::Borrowed("Upgrade"),
        _ => Cow::Borrowed(name),
    }
}

pub(super) fn header_first<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub(super) fn header_contains_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    header_first(headers, name).is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

pub(super) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}
