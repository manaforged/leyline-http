//! WebSocket client with TLS fingerprinting.
//!
//! The TLS handshake uses our fingerprinted connector (same JA3/JA4 as HTTPS).
//! The HTTP Upgrade is handled by tokio-tungstenite but over our TLS stream.
//! Chrome-accurate Origin and extension headers are injected.

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Uri};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use leyline_tls::FingerprintConnector;

use crate::error::{Error, Result};

/// A connected WebSocket with TLS fingerprinting.
///
/// ```rust,ignore
/// let mut ws = session.websocket("wss://echo.websocket.org").await?;
/// ws.send("hello").await?;
/// let msg = ws.recv().await?;
/// ws.close().await?;
/// ```
pub struct WsConnection {
    inner: WebSocketStream<tokio_boring::SslStream<tokio::net::TcpStream>>,
}

impl WsConnection {
    /// Connect to a WebSocket URL with TLS fingerprinting.
    pub(crate) async fn connect(
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

        // TLS connect with HTTP/1.1 ALPN (WebSocket needs h1, not h2).
        let tls_stream = connector
            .connect_h1(host, port, proxy)
            .await
            .map_err(Error::Tls)?;

        // Build the WebSocket upgrade request with Chrome-accurate headers.
        let ws_url = url.replace("wss://", "ws://"); // tungstenite expects ws://
        let uri: Uri = ws_url
            .parse()
            .map_err(|e: http::uri::InvalidUri| Error::Config(e.to_string()))?;
        let mut request = uri
            .into_client_request()
            .map_err(|e| Error::Http(format!("ws request: {e}")))?;

        // Set Chrome-accurate headers.
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

        // Perform the WebSocket handshake over our TLS stream.
        let (ws_stream, _response) = tokio_tungstenite::client_async(request, tls_stream.stream)
            .await
            .map_err(|e| Error::Http(format!("ws handshake: {e}")))?;

        Ok(Self { inner: ws_stream })
    }

    /// Send a text message.
    pub async fn send(&mut self, msg: &str) -> Result<()> {
        self.inner
            .send(Message::Text(msg.into()))
            .await
            .map_err(|e| Error::Http(format!("ws send: {e}")))
    }

    /// Send binary data.
    pub async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        self.inner
            .send(Message::Binary(data.into()))
            .await
            .map_err(|e| Error::Http(format!("ws send: {e}")))
    }

    /// Send a raw tungstenite Message.
    pub async fn send_raw(&mut self, msg: Message) -> Result<()> {
        self.inner
            .send(msg)
            .await
            .map_err(|e| Error::Http(format!("ws send: {e}")))
    }

    /// Receive the next message. Returns None on close.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        match self.inner.next().await {
            Some(Ok(msg)) => Ok(Some(msg)),
            Some(Err(e)) => Err(Error::Http(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }

    /// Send a close frame and shut down.
    pub async fn close(&mut self) -> Result<()> {
        self.inner
            .close(None)
            .await
            .map_err(|e| Error::Http(format!("ws close: {e}")))
    }
}
