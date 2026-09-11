use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, Uri};
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as WireClose, Role};

use crate::h2::client::H2ConnectStream;
use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::pool::Pool;
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
}

impl WsConnection {
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
        ws_config: &WebSocketConfig,
    ) -> Result<Self> {
        let parsed = url::Url::parse(url)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::new(Kind::Config).with_message("no host in WebSocket URL"))?;
        let port = parsed.port_or_known_default().unwrap_or(443);

        let tls_stream = connector
            .connect_h1(host, port, proxy)
            .await
            .map_err(Error::from)?;

        let ws_url = if let Some(rest) = url.strip_prefix("wss://") {
            format!("ws://{rest}")
        } else {
            url.to_string()
        };
        let uri: Uri = ws_url.parse().map_err(|e: http::uri::InvalidUri| {
            Error::new(Kind::Config).with_message(e.to_string())
        })?;
        let mut request = uri
            .into_client_request()
            .map_err(|e| Error::new(Kind::Request).with_message(format!("ws request: {e}")))?;

        let headers = request.headers_mut();
        if let Ok(val) = HeaderValue::from_str(user_agent) {
            headers.insert("User-Agent", val);
        }
        if let Ok(val) = HeaderValue::from_str(origin) {
            headers.insert("Origin", val);
        }

        for (name, value) in extra_headers {
            if is_reserved_ws_header(name) {
                continue;
            }
            let (hn, hv) = ws_header_pair(name, value)?;
            headers.insert(hn, hv);
        }

        let (ws_stream, response) = tokio_tungstenite::client_async_with_config(
            request,
            tls_stream.stream,
            Some(tungstenite_config(ws_config)),
        )
        .await
        .map_err(|e| Error::new(Kind::Request).with_message(format!("ws handshake: {e}")))?;

        let protocol = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        Ok(Self {
            inner: WsInner::H1(ws_stream),
            protocol,
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
        let parsed = url::Url::parse(url)?;
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
            method: "CONNECT".into(),
            scheme: "https".into(),
            authority,
            path,
            protocol: Some("websocket".into()),
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

        let ws_stream = WebSocketStream::from_raw_socket(
            stream,
            Role::Client,
            Some(tungstenite_config(ws_config)),
        )
        .await;
        Ok(Self {
            inner: WsInner::H2(ws_stream),
            protocol,
        })
    }

    pub(crate) fn is_h2_fallback_trigger(err: &Error) -> bool {
        err.kind() == Kind::Request
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

fn random_sec_ws_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    BASE64_STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests;
