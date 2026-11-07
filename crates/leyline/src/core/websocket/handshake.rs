use std::borrow::Cow;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use http::Version;
use http::header::SEC_WEBSOCKET_PROTOCOL;
use tokio_tungstenite::tungstenite::Error as WireError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

use crate::core::config::WebSocketOptions;
use crate::core::headers::reorder;
use crate::error::{Error, Kind, Result};
use crate::header_str::HeaderStr;
use crate::wire::{HeaderPair, Scheme, sent_on, strip_connection_specific_headers};

/// Returns the `https` URL that the handshake for a `wss` URL uses, or rejects any other scheme.
pub(crate) fn http_equivalent(url: &url::Url) -> Result<url::Url> {
    match url.scheme() {
        "wss" => {}
        "ws" => {
            return Err(Error::new(Kind::Request)
                .with_message("plaintext ws:// is not supported; use wss://"));
        }
        other => {
            return Err(Error::new(Kind::Request)
                .with_message(format!("websocket URL must use wss://, not {other}")));
        }
    }
    let mut http = url.clone();
    http.set_scheme(Scheme::Https.as_str())
        .map_err(|()| Error::new(Kind::Request).with_message("websocket URL must use wss://"))?;
    Ok(http)
}

/// The fields of one WebSocket opening handshake, shared by the HTTP/1.1 and HTTP/2 legs.
pub(crate) struct Handshake<'a> {
    pub(crate) url: &'a url::Url,
    pub(crate) proxy: Option<&'a crate::net::ProxyUrl>,
    pub(crate) user_agent: &'a str,
    pub(crate) origin: &'a str,
    pub(crate) extra_headers: &'a [(String, String)],
    pub(crate) header_order: Option<&'a [String]>,
    pub(crate) options: &'a WebSocketOptions,
}

impl Handshake<'_> {
    pub(super) fn headers(&self, version: Version, sec_key: &str) -> Result<Vec<HeaderPair>> {
        for (name, value) in self
            .extra_headers
            .iter()
            .filter(|(name, value)| forwards(version, name, value))
        {
            drop(ws_header_pair(name, value)?);
        }
        let mut headers: Vec<HeaderPair> = Vec::with_capacity(6 + self.extra_headers.len());
        if version == Version::HTTP_11 {
            headers.push(("Connection".into(), "Upgrade".into()));
            headers.push(("Upgrade".into(), "websocket".into()));
        }
        headers.extend([
            ("User-Agent".into(), self.user_agent.to_owned().into()),
            ("Origin".into(), self.origin.to_owned().into()),
            ("Sec-WebSocket-Version".into(), "13".into()),
            ("Sec-WebSocket-Key".into(), sec_key.to_owned().into()),
        ]);
        overlay_headers(&mut headers, self.extra_headers, version);
        if let Some(order) = self.header_order {
            reorder(&mut headers, order);
        }
        if version != Version::HTTP_11 {
            strip_connection_specific_headers(&mut headers)?;
        }
        Ok(headers)
    }
}

pub(super) fn overlay_headers(
    request: &mut Vec<HeaderPair>,
    extra: &[(String, String)],
    version: Version,
) {
    let defaults = request.len();
    for (name, value) in extra
        .iter()
        .filter(|(name, value)| forwards(version, name, value))
    {
        match request
            .iter_mut()
            .take(defaults)
            .find(|(default, _)| default.eq_ignore_ascii_case(name))
        {
            Some(slot) => slot.1 = HeaderStr::from(value.as_str()),
            None => request.push((Cow::Owned(name.clone()), HeaderStr::from(value.as_str()))),
        }
    }
}

pub(super) fn tungstenite_config(
    cfg: &WebSocketOptions,
) -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    let mut out = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    if let Some(n) = cfg.read_buffer_size {
        out.read_buffer_size = n;
    }
    if let Some(n) = cfg.write_buffer_size {
        out.write_buffer_size = n;
    }
    if let Some(n) = cfg.max_write_buffer_size {
        out.max_write_buffer_size = n;
    }
    if let Some(n) = cfg.max_message_size {
        out.max_message_size = Some(n);
    }
    if let Some(n) = cfg.max_frame_size {
        out.max_frame_size = Some(n);
    }
    out.accept_unmasked_frames = cfg.accept_unmasked_frames;
    out
}

pub(super) fn wire_error(op: &'static str, e: WireError) -> Error {
    let kind = match &e {
        WireError::Capacity(_) | WireError::Protocol(_) | WireError::Utf8 => Kind::Body,
        _ => Kind::Io,
    };
    let err = Error::new(kind).with_message(format!("{op}: {e}"));
    match e {
        WireError::Io(io) => err.with_source(io),
        other => err.with_source(other),
    }
}

pub(super) fn ws_header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue)> {
    let hn = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
        Error::new(Kind::Request)
            .with_message(format!("invalid websocket header name: {name}"))
            .with_source(e)
    })?;
    let hv = HeaderValue::from_str(value).map_err(|e| {
        Error::new(Kind::Request)
            .with_message(format!("invalid websocket header value for {name}"))
            .with_source(e)
    })?;
    Ok((hn, hv))
}

const HANDSHAKE_OWNED: [&str; 8] = [
    "host",
    "connection",
    "upgrade",
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-extensions",
    "content-length",
    "transfer-encoding",
];

fn is_reserved_ws_header(name: &str) -> bool {
    HANDSHAKE_OWNED
        .iter()
        .any(|owned| name.eq_ignore_ascii_case(owned))
}

pub(super) fn forwards(version: Version, name: &str, value: &str) -> bool {
    !is_reserved_ws_header(name) && sent_on(version, name, value)
}

fn fail(m: String) -> Result<()> {
    Err(Error::new(Kind::Request).with_message(format!("ws handshake: {m}")))
}

pub(super) fn check_upgrade_response(
    status: u16,
    headers: &[(String, String)],
    sec_key: &str,
    request: &[(String, String)],
) -> Result<()> {
    if status != 101 {
        return fail(format!("expected 101 Switching Protocols, got {status}"));
    }
    if !crate::util::header_contains_token(headers, "upgrade", "websocket")
        || !crate::util::header_contains_token(headers, "connection", "upgrade")
    {
        return fail("missing Upgrade: websocket or Connection: Upgrade".into());
    }
    if crate::util::header_first(headers, "sec-websocket-accept")
        != Some(&derive_accept_key(sec_key.as_bytes()))
    {
        return fail("Sec-WebSocket-Accept does not match the key".into());
    }
    if crate::util::header_first(headers, "sec-websocket-extensions").is_some() {
        return fail("server selected an extension the client did not offer".into());
    }
    check_subprotocol(headers, request)
}

pub(super) fn check_subprotocol(
    headers: &[(String, String)],
    request: &[(String, String)],
) -> Result<()> {
    let offered: Vec<&str> = crate::util::header_values(request, SEC_WEBSOCKET_PROTOCOL.as_str())
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect();
    let mut selected =
        crate::util::header_values(headers, SEC_WEBSOCKET_PROTOCOL.as_str()).map(str::trim);
    match (selected.next(), selected.next()) {
        (Some(_), Some(_)) => fail("server sent more than one subprotocol".into()),
        (Some(chosen), None) if !offered.contains(&chosen) => fail(format!(
            "server selected subprotocol {chosen:?}, which the client did not offer"
        )),
        (None, _) if !offered.is_empty() => {
            fail("client offered subprotocols and the server selected none".into())
        }
        _ => Ok(()),
    }
}

pub(super) fn random_sec_ws_key() -> String {
    BASE64_STANDARD.encode(crate::util::random_bytes(16))
}
