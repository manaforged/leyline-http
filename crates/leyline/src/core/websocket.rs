//! WebSocket client with TLS fingerprinting.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
pub use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, Uri};
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::h2::client::H2ConnectStream;
use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::pool::Pool;
use crate::tls::{FingerprintConnector, TlsIo};

use crate::core::error::{Error, Result};

/// Transport variant carried inside a connected [`WsConnection`].
enum WsInner {
    /// Classic HTTP/1.1 Upgrade (RFC 6455) over TLS.
    H1(WebSocketStream<TlsIo>),
    /// RFC 8441 extended CONNECT over HTTP/2.
    H2(WebSocketStream<H2ConnectStream>),
}

/// A connected WebSocket.
pub struct WsConnection {
    inner: WsInner,
    /// The subprotocol the origin selected in its handshake response (`Sec-WebSocket-Protocol`), if any.
    protocol: Option<String>,
}

impl WsConnection {
    /// Connect over HTTP/1.1.
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
    ) -> Result<Self> {
        let parsed = url::Url::parse(url)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::Config("no host in WebSocket URL".into()))?;
        let port = parsed.port_or_known_default().unwrap_or(443);

        let tls_stream = connector
            .connect_h1(host, port, proxy)
            .await
            .map_err(Error::Tls)?;

        let ws_url = if let Some(rest) = url.strip_prefix("wss://") {
            format!("ws://{rest}")
        } else {
            url.to_string()
        };
        let uri: Uri = ws_url
            .parse()
            .map_err(|e: http::uri::InvalidUri| Error::Config(e.to_string()))?;
        let mut request = uri
            .into_client_request()
            .map_err(|e| Error::Http(format!("ws request: {e}")))?;

        let headers = request.headers_mut();
        if let Ok(val) = HeaderValue::from_str(user_agent) {
            headers.insert("User-Agent", val);
        }
        if let Ok(val) = HeaderValue::from_str(origin) {
            headers.insert("Origin", val);
        }
        headers.insert(
            "Sec-WebSocket-Extensions",
            HeaderValue::from_static("permessage-deflate; client_max_window_bits"),
        );

        for (name, value) in extra_headers {
            if is_reserved_ws_header(name) {
                continue;
            }
            let (hn, hv) = ws_header_pair(name, value)?;
            headers.insert(hn, hv);
        }

        let (ws_stream, response) = tokio_tungstenite::client_async(request, tls_stream.stream)
            .await
            .map_err(|e| Error::Http(format!("ws handshake: {e}")))?;

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

    /// Connect over HTTP/2 extended CONNECT (RFC 8441).
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
    ) -> Result<Self> {
        let parsed = url::Url::parse(url)?;
        let host = parsed
            .host_str()
            .ok_or_else(|| Error::Config("no host in WebSocket URL".into()))?
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
            return Err(Error::Http(H2_NO_CONNECT_PROTOCOL.into()));
        }

        let sec_key = random_sec_ws_key();
        let mut headers: Vec<(String, String)> = Vec::with_capacity(8);
        headers.push(("sec-websocket-version".into(), "13".into()));
        headers.push(("sec-websocket-key".into(), sec_key));
        headers.push((
            "sec-websocket-extensions".into(),
            "permessage-deflate; client_max_window_bits".into(),
        ));
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
            .map_err(|e| Error::Http(format!("h2 ws: {e}")))?;

        if stream.status() != 200 {
            return Err(Error::Http(format!(
                "ws handshake: h2 extended CONNECT returned :status {}",
                stream.status()
            )));
        }

        let protocol = stream
            .response_headers()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("sec-websocket-protocol"))
            .map(|(_, v)| v.clone());

        let ws_stream = WebSocketStream::from_raw_socket(stream, Role::Client, None).await;
        Ok(Self {
            inner: WsInner::H2(ws_stream),
            protocol,
        })
    }

    /// True if `err` is the sentinel "peer doesn't enable CONNECT protocol" failure from [`connect_h2`](Self::connect_h2).
    pub(crate) fn is_h2_fallback_trigger(err: &Error) -> bool {
        matches!(err, Error::Http(s) if s.contains(H2_NO_CONNECT_PROTOCOL))
    }

    /// Send a text message.
    pub async fn send(&mut self, msg: &str) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(Message::Text(msg.into()))
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(Message::Text(msg.into()))
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
        }
    }

    /// Send binary data.
    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(Message::Binary(data.into()))
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(Message::Binary(data.into()))
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
        }
    }

    /// Send a raw [`WsMessage`].
    pub async fn send_raw(&mut self, msg: WsMessage) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .send(msg)
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
            WsInner::H2(s) => s
                .send(msg)
                .await
                .map_err(|e| Error::Http(format!("ws send: {e}"))),
        }
    }

    /// Receive the next message.
    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsInner::H1(s) => s.next().await,
            WsInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(msg)),
            Some(Err(e)) => Err(Error::Http(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }

    /// Send a close frame and shut down.
    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s
                .close(None)
                .await
                .map_err(|e| Error::Http(format!("ws close: {e}"))),
            WsInner::H2(s) => s
                .close(None)
                .await
                .map_err(|e| Error::Http(format!("ws close: {e}"))),
        }
    }

    /// `true` if the underlying transport is HTTP/2 extended CONNECT.
    pub fn is_http2(&self) -> bool {
        matches!(self.inner, WsInner::H2(_))
    }

    /// The subprotocol the origin selected in its handshake response (`Sec-WebSocket-Protocol`), if any.
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    /// Split into independent send and receive halves so each direction can be driven by its own task.
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

/// Write half of a split [`WsConnection`].
enum WsSinkInner {
    H1(SplitSink<WebSocketStream<TlsIo>, Message>),
    H2(SplitSink<WebSocketStream<H2ConnectStream>, Message>),
}

/// The send half returned by [`WsConnection::split`].
pub struct WsSink {
    inner: WsSinkInner,
}

impl WsSink {
    /// Send a text message.
    pub async fn send(&mut self, msg: &str) -> Result<()> {
        self.send_raw(Message::Text(msg.into())).await
    }

    /// Send binary data.
    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        self.send_raw(Message::Binary(data.into())).await
    }

    /// Send a raw [`WsMessage`].
    pub async fn send_raw(&mut self, msg: WsMessage) -> Result<()> {
        match &mut self.inner {
            WsSinkInner::H1(s) => s.send(msg).await,
            WsSinkInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| Error::Http(format!("ws send: {e}")))
    }

    /// Send a close frame and shut the write half down.
    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsSinkInner::H1(s) => s.close().await,
            WsSinkInner::H2(s) => s.close().await,
        }
        .map_err(|e| Error::Http(format!("ws close: {e}")))
    }
}

/// Read half of a split [`WsConnection`].
enum WsStreamInner {
    H1(SplitStream<WebSocketStream<TlsIo>>),
    H2(SplitStream<WebSocketStream<H2ConnectStream>>),
}

/// The receive half returned by [`WsConnection::split`].
pub struct WsStream {
    inner: WsStreamInner,
}

impl WsStream {
    /// Receive the next message.
    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsStreamInner::H1(s) => s.next().await,
            WsStreamInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(msg)),
            Some(Err(e)) => Err(Error::Http(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }
}

/// Sentinel message embedded in the `Error::Http` string when the peer's pooled H2 connection doesn't advertise `SETTINGS_ENABLE_CONNECT_PROTOCOL`.
const H2_NO_CONNECT_PROTOCOL: &str = "h2-no-connect-protocol";

fn ws_header_pair(name: &str, value: &str) -> Result<(HeaderName, HeaderValue)> {
    let hn = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| Error::Http(format!("invalid websocket header name: {name}")))?;
    let hv = HeaderValue::from_str(value)
        .map_err(|_| Error::Http(format!("invalid websocket header value for {name}")))?;
    Ok((hn, hv))
}

/// Headers the WebSocket handshake owns.
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

/// Generate a cryptographically random 16-byte `Sec-WebSocket-Key` (RFC 6455 §4.1).
fn random_sec_ws_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    BASE64_STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests;
