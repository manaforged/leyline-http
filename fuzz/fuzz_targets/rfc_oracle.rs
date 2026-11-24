#![allow(
    dead_code,
    reason = "shared oracle helpers used across multiple fuzz targets"
)]

pub(crate) fn is_tchar(b: u8) -> bool {
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

pub(crate) fn next_strict_crlf(buf: &[u8], max_line: usize) -> Option<usize> {
    let lf = buf.iter().position(|&b| b == b'\n')?;
    if lf == 0 || buf[lf - 1] != b'\r' {
        return None;
    }
    let pos = lf - 1;
    (pos <= max_line).then_some(pos)
}

pub(crate) fn split_crlf_line(buf: &[u8]) -> (&[u8], &[u8]) {
    match buf.windows(2).position(|w| w == b"\r\n") {
        Some(end) => (&buf[..end], &buf[end + 2..]),
        None => (buf, &[]),
    }
}

pub(crate) fn trim_ows(mut val: &[u8]) -> &[u8] {
    while let Some((&b, rest)) = val.split_first() {
        if matches!(b, b' ' | b'\t') {
            val = rest;
        } else {
            break;
        }
    }
    while let Some((&b, rest)) = val.split_last() {
        if matches!(b, b' ' | b'\t') {
            val = rest;
        } else {
            break;
        }
    }
    val
}
