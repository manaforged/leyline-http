pub(super) fn contains_header<K: AsRef<str>, V>(headers: &[(K, V)], name: &str) -> bool {
    headers
        .iter()
        .any(|(k, _)| k.as_ref().eq_ignore_ascii_case(name))
}

pub(super) fn is_valid_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| TOKEN[usize::from(b)])
}

static TOKEN: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = token_byte(i as u8);
        i += 1;
    }
    table
};

const fn token_byte(b: u8) -> bool {
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
}

pub(super) fn is_valid_header_value(s: &str) -> bool {
    s.bytes().fold(true, |valid, b| {
        valid & ((b >= b' ') & (b != 0x7F) | (b == b'\t'))
    })
}

pub(super) fn is_valid_request_target(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b > 0x20 && b != 0x7F)
}

pub(super) fn h1_header_name(name: &str) -> &str {
    match name {
        _ if name.eq_ignore_ascii_case("host") => "Host",
        _ if name.eq_ignore_ascii_case("connection") => "Connection",
        _ if name.eq_ignore_ascii_case("user-agent") => "User-Agent",
        _ if name.eq_ignore_ascii_case("accept") => "Accept",
        _ if name.eq_ignore_ascii_case("accept-encoding") => "Accept-Encoding",
        _ if name.eq_ignore_ascii_case("accept-language") => "Accept-Language",
        _ if name.eq_ignore_ascii_case("content-length") => "Content-Length",
        _ if name.eq_ignore_ascii_case("content-type") => "Content-Type",
        _ if name.eq_ignore_ascii_case("cookie") => "Cookie",
        _ if name.eq_ignore_ascii_case("authorization") => "Authorization",
        _ if name.eq_ignore_ascii_case("proxy-authorization") => "Proxy-Authorization",
        _ if name.eq_ignore_ascii_case("origin") => "Origin",
        _ if name.eq_ignore_ascii_case("referer") => "Referer",
        _ if name.eq_ignore_ascii_case("upgrade") => "Upgrade",
        _ => name,
    }
}

pub(super) fn find_header_end(buf: &[u8]) -> Option<usize> {
    memchr::memchr_iter(b'\n', buf)
        .find(|&i| i >= 3 && &buf[i - 3..=i] == b"\r\n\r\n")
        .map(|i| i - 3)
}
