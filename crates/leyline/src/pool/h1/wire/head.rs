use super::*;
use crate::wire::BodyKind;

pub(in crate::pool::h1) fn invalid_method(method: &str) -> Error {
    Error::config(format!(
        "invalid HTTP method `{method}`: non-token bytes not allowed"
    ))
}

pub(in crate::pool::h1) fn validate<K: AsRef<str>, V: AsRef<str>>(
    method: &Method,
    headers: &[(K, V)],
) -> Result<(), Error> {
    if !is_valid_token(method.as_str()) {
        return Err(invalid_method(method.as_str()));
    }
    for (name, value) in headers {
        let (name, value) = (name.as_ref(), value.as_ref());
        if !is_valid_token(name) {
            return Err(Error::config(format!(
                "invalid header name `{name}`: non-token bytes not allowed"
            )));
        }
        if !is_valid_header_value(value) {
            return Err(Error::config(format!(
                "invalid value for header `{name}`: control characters not allowed"
            )));
        }
    }
    Ok(())
}

pub(super) fn target(
    url: &url::Url,
    target: H1Target,
) -> Result<(Cow<'_, str>, Cow<'_, str>), Error> {
    let (host, port) = crate::wire::host_port(url)?;
    let authority = crate::wire::authority(url, host, port);
    let path = crate::util::request_target(url);
    let request_target = match target {
        H1Target::OriginForm => Cow::Borrowed(path),
        H1Target::AbsoluteForm => Cow::Owned(format!("{}://{}{}", url.scheme(), authority, path)),
    };
    if !is_valid_request_target(&request_target) {
        return Err(Error::config(
            "invalid request target: control characters not allowed",
        ));
    }
    Ok((request_target, authority))
}

pub(super) type Length = Option<(&'static str, Cow<'static, str>)>;

pub(super) fn frame(method: &Method, headers: &[HeaderPair], body: Body) -> (Framing, Length) {
    let has_cl = contains_header(headers, "content-length");
    let has_te = contains_header(headers, "transfer-encoding");
    let length = |n: String| (!has_cl && !has_te).then_some(("Content-Length", Cow::Owned(n)));
    match body.0 {
        BodyKind::Empty => {
            let zero = crate::wire::has_body_by_default(method) && !has_cl && !has_te;
            (
                Framing::None,
                zero.then_some(("Content-Length", Cow::Borrowed("0"))),
            )
        }
        BodyKind::Bytes(b) => {
            let length = length(b.len().to_string());
            (Framing::Buffered(b), length)
        }
        BodyKind::Stream {
            stream,
            length_hint: Some(n),
        } => (Framing::Fixed { stream }, length(n.to_string())),
        BodyKind::Stream {
            stream,
            length_hint: None,
        } => (
            Framing::Chunked { stream },
            (!has_te).then_some(("Transfer-Encoding", Cow::Borrowed("chunked"))),
        ),
    }
}

pub(super) struct Head<'a> {
    pub(super) method: &'a str,
    pub(super) target: &'a str,
    pub(super) host: Option<&'a str>,
    pub(super) headers: &'a [HeaderPair],
    pub(super) lines: Option<&'a [u8]>,
    pub(super) length: Option<(&'a str, &'a str)>,
    pub(super) keep_alive: bool,
}

impl Head<'_> {
    pub(super) fn encode(&self, req: &mut Vec<u8>) {
        let line = |(name, value): (&str, &str)| name.len() + value.len() + 4;
        let size = self.method.len()
            + self.target.len()
            + 13
            + self.host.map_or(0, |host| line(("Host", host)))
            + self.lines.map_or_else(
                || {
                    self.headers
                        .iter()
                        .map(|(name, value)| line((name.as_ref(), value.as_ref())))
                        .sum::<usize>()
                },
                <[u8]>::len,
            )
            + self.length.map_or(0, line)
            + 26;
        req.clear();
        req.reserve(size);
        req.extend_from_slice(self.method.as_bytes());
        req.push(b' ');
        req.extend_from_slice(self.target.as_bytes());
        req.extend_from_slice(b" HTTP/1.1\r\n");
        if let Some(host) = self.host {
            put(req, "Host", host);
        }
        match self.lines {
            Some(lines) => req.extend_from_slice(lines),
            None => {
                for (name, value) in self.headers {
                    put(req, h1_header_name(name), value);
                }
            }
        }
        if let Some((name, value)) = self.length {
            put(req, name, value);
        }
        if self.keep_alive {
            put(req, "Connection", "keep-alive");
        }
        req.extend_from_slice(b"\r\n");
    }
}
