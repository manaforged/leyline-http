//! WebSocket client with TLS fingerprinting.
//!
//! Two transports share one caller-facing type:
//!
//! - **HTTP/1.1 Upgrade** (RFC 6455) — the long-standing path. A fresh
//!   TLS connection is negotiated with `http/1.1` ALPN and
//!   tokio-tungstenite drives the Upgrade handshake over it.
//! - **HTTP/2 extended CONNECT** (RFC 8441) — when the session's pooled
//!   H2 connection to the destination advertises
//!   `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1`, the WebSocket opens as a
//!   bidirectional stream inside that H2 connection: the handshake
//!   happens in HEADERS, DATA frames carry WebSocket frames.
//!
//! Both return the same [`WsConnection`] so callers don't see the
//! difference. The HTTP/1.1 variant remains the fallback whenever ALPN
//! doesn't negotiate `h2` or the peer hasn't enabled extended CONNECT.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Uri};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use leyline_h2::client::H2ConnectStream;
use leyline_h2::config::H2Config;
use leyline_h2::connection::PseudoHeaders;
use leyline_pool::Pool;
use leyline_tls::FingerprintConnector;

use crate::error::{Error, Result};

/// Transport variant carried inside a connected [`WsConnection`].
///
/// Boxed because the two sides have different sizes (an H1 TLS stream
/// is a concrete `SslStream<TcpStream>`; an H2 CONNECT stream is an
/// `H2ConnectStream`). Keeping the enum behind an inner field also
/// lets us evolve the list without breaking the public `WsConnection`
/// API.
enum WsInner {
    /// Classic HTTP/1.1 Upgrade (RFC 6455) over TLS.
    H1(WebSocketStream<tokio_boring::SslStream<tokio::net::TcpStream>>),
    /// RFC 8441 extended CONNECT over HTTP/2.
    H2(WebSocketStream<H2ConnectStream>),
}

/// A connected WebSocket.
///
/// ```rust,ignore
/// let mut ws = session.websocket("wss://echo.example.com/ws").await?;
/// ws.send("hello").await?;
/// let msg = ws.recv().await?;
/// ws.close().await?;
/// ```
pub struct WsConnection {
    inner: WsInner,
}

impl WsConnection {
    /// Connect over HTTP/1.1. This is the legacy path — a fresh TLS
    /// connection is established with `http/1.1` ALPN and
    /// tokio-tungstenite performs the Upgrade handshake.
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
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

        let ws_url = url.replace("wss://", "ws://");
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

        let (ws_stream, _response) = tokio_tungstenite::client_async(request, tls_stream.stream)
            .await
            .map_err(|e| Error::Http(format!("ws handshake: {e}")))?;

        Ok(Self {
            inner: WsInner::H1(ws_stream),
        })
    }

    /// Connect over HTTP/2 extended CONNECT (RFC 8441).
    ///
    /// Uses the session pool: if a pooled H2 connection for the
    /// destination exists it is reused, otherwise a fresh one is
    /// handshaked. Returns an error shaped so that
    /// [`WsConnection::is_h2_fallback_trigger`] picks it up when the
    /// peer has not advertised `SETTINGS_ENABLE_CONNECT_PROTOCOL=1`.
    pub(crate) async fn connect_h2(
        pool: &Arc<Pool>,
        connector: &FingerprintConnector,
        h2_config: &H2Config,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
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

        // Grab (or open) a pooled H2 handle for this destination.
        let (h2_client, _tls) =
            leyline_pool::checkout_handle(pool, connector, h2_config, &host, port, proxy)
                .await
                .map_err(|e| Error::Http(format!("h2 pool: {e}")))?;

        if !h2_client.peer_enables_connect_protocol() {
            return Err(Error::Http(H2_NO_CONNECT_PROTOCOL.into()));
        }

        // Build the WebSocket handshake headers. RFC 8441 §4 reuses
        // the RFC 6455 Sec-WebSocket-* headers on the HEADERS frame.
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

        let pseudo = PseudoHeaders {
            method: "CONNECT".into(),
            scheme: "https".into(),
            authority,
            path,
            protocol: Some("websocket".into()),
        };

        let stream = h2_client
            .open_extended_connect(pseudo, headers)
            .await
            .map_err(|e| Error::Http(format!("h2 ws: {e}")))?;

        // RFC 8441 §5: :status 200 is success; anything else is a
        // handshake failure. Surface 4xx/5xx before any WebSocket
        // framing code touches the stream.
        if stream.status() != 200 {
            return Err(Error::Http(format!(
                "ws handshake: h2 extended CONNECT returned :status {}",
                stream.status()
            )));
        }

        // With the handshake already done on HEADERS, the bidirectional
        // stream carries only WebSocket frames — exactly what
        // `from_raw_socket` is for.
        let ws_stream = WebSocketStream::from_raw_socket(stream, Role::Client, None).await;
        Ok(Self {
            inner: WsInner::H2(ws_stream),
        })
    }

    /// True if `err` is the sentinel "peer doesn't enable CONNECT
    /// protocol" failure from [`connect_h2`](Self::connect_h2). The
    /// session uses this to decide whether an H2 attempt should fall
    /// back to a fresh H1 upgrade.
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

    /// Send a raw tungstenite Message.
    pub async fn send_raw(&mut self, msg: Message) -> Result<()> {
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

    /// Receive the next message. Returns None on close.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
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
}

/// Sentinel message embedded in the `Error::Http` string when the
/// peer's pooled H2 connection doesn't advertise
/// `SETTINGS_ENABLE_CONNECT_PROTOCOL`. Matched by
/// [`WsConnection::is_h2_fallback_trigger`] to drive the H1 fallback.
const H2_NO_CONNECT_PROTOCOL: &str = "h2-no-connect-protocol";

/// Generate a random 16-byte Sec-WebSocket-Key (RFC 6455 §4.1). We
/// avoid pulling in the `rand` crate for one helper — the workspace
/// already uses `std::time::UNIX_EPOCH` / `hash` for similar tasks and
/// the key is only used for handshake matching, not security.
fn random_sec_ws_key() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut bytes = [0u8; 16];
    let mut filled = 0;
    while filled < bytes.len() {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        );
        hasher.write_u64(filled as u64);
        let h = hasher.finish().to_le_bytes();
        let n = (bytes.len() - filled).min(8);
        bytes[filled..filled + n].copy_from_slice(&h[..n]);
        filled += n;
    }
    BASE64_STANDARD.encode(bytes)
}
