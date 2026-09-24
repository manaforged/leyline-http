use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as WireClose, Role};

use crate::core::headers::reorder;
use crate::h2::client::H2ConnectStream;
use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::Pool;
use crate::profile::preset::HeaderPair;
use crate::tls::{FingerprintConnector, TlsIo};

use crate::core::WebSocketConfig;
use crate::core::error::{Error, Kind, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close(Option<CloseFrame>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CloseFrame {
    pub code: u16,
    pub reason: String,
}

impl CloseFrame {
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl WsMessage {
    fn wire(msg: Message) -> Self {
        match msg {
            Message::Text(s) => Self::Text(s.as_str().to_owned()),
            Message::Binary(b) => Self::Binary(b.to_vec()),
            Message::Ping(_) => Self::Ping,
            Message::Pong(_) => Self::Pong,
            Message::Close(frame) => Self::Close(frame.map(|f| CloseFrame {
                code: f.code.into(),
                reason: f.reason.as_str().to_owned(),
            })),
            Message::Frame(f) => Self::Binary(f.into_payload().to_vec()),
        }
    }

    fn into_wire(self) -> Message {
        match self {
            Self::Text(s) => Message::Text(s.into()),
            Self::Binary(b) => Message::Binary(b.into()),
            Self::Ping => Message::Ping(Vec::new().into()),
            Self::Pong => Message::Pong(Vec::new().into()),
            Self::Close(frame) => Message::Close(frame.map(|f| WireClose {
                code: f.code.into(),
                reason: f.reason.into(),
            })),
        }
    }
}

enum WsInner {
    H1(WebSocketStream<TlsIo>),
    H2(WebSocketStream<H2ConnectStream>),
}

pub struct WsConnection {
    inner: WsInner,
    protocol: Option<String>,
    headers: Vec<(String, String)>,
}

impl WsConnection {
    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
        header_order: Option<&[String]>,
        ws_config: &WebSocketConfig,
    ) -> Result<Self> {
        let mut parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::new(Kind::Config).with_message("no host in WebSocket URL"))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);
        parsed
            .set_scheme("https")
            .map_err(|()| Error::new(Kind::Config).with_message("WebSocket URL must use wss://"))?;

        let mut stream = connector
            .connect_h1(&host, port, proxy)
            .await
            .map_err(Error::from)?
            .stream;

        let sec_key = random_sec_ws_key();
        let mut request: Vec<HeaderPair> = vec![
            ("Connection".into(), "Upgrade".into()),
            ("Upgrade".into(), "websocket".into()),
            ("User-Agent".into(), user_agent.to_owned().into()),
            ("Origin".into(), origin.to_owned().into()),
            ("Sec-WebSocket-Version".into(), "13".into()),
            ("Sec-WebSocket-Key".into(), sec_key.clone().into()),
        ];
        for (name, value) in extra_headers {
            if is_reserved_ws_header(name) {
                continue;
            }
            match request
                .iter_mut()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                Some(slot) => slot.1 = value.clone().into(),
                None => request.push((name.clone().into(), value.clone().into())),
            }
        }
        if let Some(order) = header_order {
            reorder(&mut request, order);
        }
        let request = request
            .into_iter()
            .map(|(n, v)| (n.into_owned(), v.into_owned()))
            .collect();

        let ((status, headers, _), leftover) =
            crate::pool::upgrade_on_stream(&mut stream, &parsed, request)
                .await
                .map_err(crate::core::transport::h1_error_to_core)?;
        check_upgrade_response(status, &headers, &sec_key)?;

        let protocol = response_header(&headers, "sec-websocket-protocol").map(str::to_owned);
        let ws_stream = WebSocketStream::from_partially_read(
            stream,
            leftover,
            Role::Client,
            Some(tungstenite_config(ws_config)),
        )
        .await;

        Ok(Self {
            inner: WsInner::H1(ws_stream),
            protocol,
            headers,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub(crate) async fn connect_h2(
        pool: &Arc<Pool>,
        connector: &FingerprintConnector,
        h2_config: &H2Config,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
        ws_config: &WebSocketConfig,
    ) -> Result<Self> {
        let parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::new(Kind::Config).with_message("no host in WebSocket URL"))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);
        let path = {
            let mut p = parsed.path().to_string();
            if p.is_empty() {
                p.push('/');
            }
            if let Some(q) = parsed.query() {
                p.push('?');
                p.push_str(q);
            }
            p
        };
        let authority = if port == 443 {
            host.clone()
        } else {
            format!("{host}:{port}")
        };

        let (h2_client, _tls) =
            crate::pool::checkout_handle(pool, connector, h2_config, &host, port, proxy).await?;

        if !h2_client.peer_enables_connect_protocol() {
            return Err(Error::new(Kind::Request).with_message(H2_NO_CONNECT_PROTOCOL));
        }

        let sec_key = random_sec_ws_key();
        let mut headers: Vec<(String, String)> = Vec::with_capacity(8);
        headers.push(("sec-websocket-version".into(), "13".into()));
        headers.push(("sec-websocket-key".into(), sec_key));
        headers.push(("user-agent".into(), user_agent.into()));
        headers.push(("origin".into(), origin.into()));

        for (name, value) in extra_headers {
            let lname = name.to_ascii_lowercase();
            if is_reserved_ws_header(&lname) {
                continue;
            }
            drop(ws_header_pair(name, value)?);
            match headers.iter_mut().find(|(n, _)| n == &lname) {
                Some(slot) => slot.1 = value.clone(),
                None => headers.push((lname, value.clone())),
            }
        }

        let pseudo = PseudoHeaders {
            method: HeaderStr::from_static("CONNECT"),
            scheme: HeaderStr::from_static("https"),
            authority: HeaderStr::from(authority),
            path: HeaderStr::from(path),
            protocol: Some(HeaderStr::from_static("websocket")),
        };

        let stream = h2_client
            .open_extended_connect(
                pseudo,
                headers
                    .into_iter()
                    .map(|(k, v)| (std::borrow::Cow::Owned(k), std::borrow::Cow::Owned(v)))
                    .collect(),
            )
            .await
            .map_err(|e| Error::new(Kind::Request).with_message(format!("h2 ws: {e}")))?;

        if stream.status() != 200 {
            return Err(Error::new(Kind::Request).with_message(format!(
                "ws handshake: h2 extended CONNECT returned :status {}",
                stream.status()
            )));
        }

        let protocol = stream
            .response_headers()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("sec-websocket-protocol"))
            .map(|(_, v)| v.clone());
        let headers = stream.response_headers().to_vec();

        let ws_stream = WebSocketStream::from_raw_socket(
            stream,
            Role::Client,
            Some(tungstenite_config(ws_config)),
        )
        .await;
        Ok(Self {
            inner: WsInner::H2(ws_stream),
            protocol,
            headers,
        })
    }

    pub(crate) fn is_h2_fallback_trigger(err: &Error) -> bool {
        crate::core::transport::is_h2_alpn_mismatch(err)
            || err.kind() == Kind::Request
                && err
                    .message()
                    .is_some_and(|s| s.contains(H2_NO_CONNECT_PROTOCOL))
    }

    pub async fn send(&mut self, msg: &str) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(Message::Text(msg.into()))
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(Message::Text(msg.into()))
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
        }
    }

    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(Message::Binary(data.into()))
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(Message::Binary(data.into()))
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
        }
    }

    pub async fn send_raw(&mut self, msg: WsMessage) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(msg)
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(msg)
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}"))),
        }
    }

    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsInner::H1(s) => s.next().await,
            WsInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(WsMessage::wire(msg))),
            Some(Err(e)) => Err(Error::new(Kind::Request).with_message(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .close(None)
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws close: {e}"))),
            WsInner::H2(s) => s
                .close(None)
                .await
                .map_err(|e| Error::new(Kind::Request).with_message(format!("ws close: {e}"))),
        }
    }

    pub fn is_http2(&self) -> bool {
        matches!(self.inner, WsInner::H2(_))
    }

    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn split(self) -> (WsSink, WsStream) {
        match self.inner {
            WsInner::H1(s) => {
                let (tx, rx) = s.split();
                (
                    WsSink {
                        inner: WsSinkInner::H1(tx),
                    },
                    WsStream {
                        inner: WsStreamInner::H1(rx),
                    },
                )
            }
            WsInner::H2(s) => {
                let (tx, rx) = s.split();
                (
                    WsSink {
                        inner: WsSinkInner::H2(tx),
                    },
                    WsStream {
                        inner: WsStreamInner::H2(rx),
                    },
                )
            }
        }
    }
}

enum WsSinkInner {
    H1(SplitSink<WebSocketStream<TlsIo>, Message>),
    H2(SplitSink<WebSocketStream<H2ConnectStream>, Message>),
}

pub struct WsSink {
    inner: WsSinkInner,
}

impl WsSink {
    pub async fn send(&mut self, msg: &str) -> Result<()> {
        self.send_raw(WsMessage::Text(msg.to_owned())).await
    }

    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        self.send_raw(WsMessage::Binary(data)).await
    }

    pub async fn send_raw(&mut self, msg: WsMessage) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsSinkInner::H1(s) => s.send(msg).await,
            WsSinkInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}")))
    }

    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsSinkInner::H1(s) => s.close().await,
            WsSinkInner::H2(s) => s.close().await,
        }
        .map_err(|e| Error::new(Kind::Request).with_message(format!("ws close: {e}")))
    }
}

enum WsStreamInner {
    H1(SplitStream<WebSocketStream<TlsIo>>),
    H2(SplitStream<WebSocketStream<H2ConnectStream>>),
}

pub struct WsStream {
    inner: WsStreamInner,
}

impl WsStream {
    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsStreamInner::H1(s) => s.next().await,
            WsStreamInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(WsMessage::wire(msg))),
            Some(Err(e)) => Err(Error::new(Kind::Request).with_message(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }
}

const H2_NO_CONNECT_PROTOCOL: &str = "h2-no-connect-protocol";

fn tungstenite_config(
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

fn ws_header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue)> {
    let hn = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        Error::new(Kind::Request).with_message(format!("invalid websocket header name: {name}"))
    })?;
    let hv = HeaderValue::from_str(value).map_err(|_| {
        Error::new(Kind::Request).with_message(format!("invalid websocket header value for {name}"))
    })?;
    Ok((hn, hv))
}

fn is_reserved_ws_header(name: &str) -> bool {
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

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn has_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    response_header(headers, name).is_some_and(|v| {
        v.split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

fn check_upgrade_response(status: u16, headers: &[(String, String)], sec_key: &str) -> Result<()> {
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

fn random_sec_ws_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    BASE64_STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests;
