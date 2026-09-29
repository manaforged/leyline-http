use std::borrow::Cow;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use tokio_tungstenite::tungstenite::Error as WireError;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

use crate::core::WebSocketConfig;
use crate::core::error::{Error, Kind, Result};
use crate::profile::preset::HeaderPair;

pub(super) const H2_NO_CONNECT_PROTOCOL: &str = "h2-no-connect-protocol";

pub(super) fn handshake_target(url: &str) -> Result<(url::Url, String, u16)> {
    let mut parsed = url::Url::parse(url).map_err(Error::from_url_parse)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in WebSocket URL"))?
        .to_owned();
    let port = parsed.port_or_known_default().unwrap_or(443);
    parsed
        .set_scheme("https")
        .map_err(|()| Error::new(Kind::Config).with_message("WebSocket URL must use wss://"))?;
    Ok((parsed, host, port))
}

pub(super) fn overlay_headers(request: &mut Vec<HeaderPair>, extra: &[(String, String)]) {
    let defaults = request.len();
    for (name, value) in extra
        .iter()
        .filter(|(name, _)| !is_reserved_ws_header(name))
    {
        match request
            .iter_mut()
            .take(defaults)
            .find(|(default, _)| default.eq_ignore_ascii_case(name))
        {
            Some(slot) => slot.1 = Cow::Owned(value.clone()),
            None => request.push((Cow::Owned(name.clone()), Cow::Owned(value.clone()))),
        }
    }
}

pub(super) fn tungstenite_config(
    cfg: &WebSocketConfig,
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
    let hn = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        Error::new(Kind::Request).with_message(format!("invalid websocket header name: {name}"))
    })?;
    let hv = HeaderValue::from_str(value).map_err(|_| {
        Error::new(Kind::Request).with_message(format!("invalid websocket header value for {name}"))
    })?;
    Ok((hn, hv))
}

pub(super) fn is_reserved_ws_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-extensions"
            | "content-length"
    )
}

pub(super) fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

pub(super) fn has_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    response_header(headers, name).is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

pub(super) fn check_upgrade_response(
    status: u16,
    headers: &[(String, String)],
    sec_key: &str,
) -> Result<()> {
    let fail =
        |m: String| Err(Error::new(Kind::Request).with_message(format!("ws handshake: {m}")));
    if status != 101 {
        return fail(format!("expected 101 Switching Protocols, got {status}"));
    }
    if !has_token(headers, "upgrade", "websocket") || !has_token(headers, "connection", "upgrade") {
        return fail("missing Upgrade: websocket or Connection: Upgrade".into());
    }
    if response_header(headers, "sec-websocket-accept")
        != Some(&derive_accept_key(sec_key.as_bytes()))
    {
        return fail("Sec-WebSocket-Accept does not match the key".into());
    }
    if response_header(headers, "sec-websocket-extensions").is_some() {
        return fail("server selected an extension the client did not offer".into());
    }
    Ok(())
}

pub(super) fn random_sec_ws_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    BASE64_STANDARD.encode(bytes)
}
