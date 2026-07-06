use super::Session;
use crate::core::WebSocketConfig;
use crate::core::error::Result;

impl Session {
    // WebSocket.

    /// Connect to a WebSocket URL with TLS fingerprinting.
    ///
    /// Path selection is two-tier:
    ///
    /// 1. **HTTP/2 extended CONNECT** (RFC 8441) is attempted first.
    ///    It piggybacks on whatever pooled H2 connection the session
    ///    already maintains to the destination (or opens a fresh one
    ///    with the standard ALPN list). If the peer advertises
    ///    `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1` the WebSocket runs as
    ///    a bidirectional stream inside the existing H2 connection,
    ///    sharing its TLS session and TCP connection.
    /// 2. **HTTP/1.1 Upgrade** is used as a fallback when either the
    ///    peer doesn't advertise extended CONNECT, the ALPN doesn't
    ///    negotiate `h2`, or the H2 attempt errors. A fresh TLS
    ///    connection with `http/1.1` ALPN is negotiated and
    ///    tokio-tungstenite drives the classic RFC 6455 handshake.
    ///
    /// In both cases the returned [`WsConnection`](
    /// crate::core::websocket::WsConnection) looks identical to the caller.
    /// Use [`websocket_http1`](Self::websocket_http1) to force the
    /// legacy H1 path — useful for testing servers that speak both.
    ///
    /// ```rust,ignore
    /// let mut ws = session.websocket("wss://echo.example.com/ws").await?;
    /// ws.send("hello").await?;
    /// if let Some(msg) = ws.recv().await? {
    ///     println!("{}", msg);
    /// }
    /// ws.close().await?;
    /// ```
    pub async fn websocket(&self, url: &str) -> Result<crate::core::websocket::WsConnection> {
        self.websocket_builder(url).connect().await
    }

    /// Create a configurable WebSocket connector for this URL.
    pub fn websocket_builder<'a>(&'a self, url: &'a str) -> WebSocketBuilder<'a> {
        WebSocketBuilder {
            session: self,
            url,
            config: self.websocket_config,
            force_http1: false,
            proxy: None,
            headers: Vec::new(),
        }
    }

    async fn websocket_with_options(
        &self,
        url: &str,
        config: WebSocketConfig,
        force_http1: bool,
        request_proxy: Option<&str>,
        extra_headers: &[(String, String)],
    ) -> Result<crate::core::websocket::WsConnection> {
        let origin = ws_origin(url)?;
        let parsed = url::Url::parse(url)?;
        let proxy = self.proxy_config.proxy_for(
            &parsed,
            request_proxy,
            self.proxy.as_deref(),
            self.proxy_from_env,
        );

        // Try H2 first. The pool helper handles the TLS handshake +
        // ALPN check; if the connection already existed we just clone
        // its handle. Failures that look like "peer didn't enable
        // CONNECT protocol" fall through to the H1 upgrade path.
        if config.prefer_http2 && !force_http1 {
            match crate::core::websocket::WsConnection::connect_h2(
                &self.pool,
                &self.connector,
                &self.h2_config,
                url,
                proxy,
                &self.user_agent,
                &origin,
                extra_headers,
            )
            .await
            {
                Ok(conn) => return Ok(conn),
                Err(e) if crate::core::websocket::WsConnection::is_h2_fallback_trigger(&e) => {
                    tracing::debug!(
                        error = %e,
                        "H2 extended CONNECT not available, falling back to H1 Upgrade"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        "H2 WebSocket path failed, falling back to H1 Upgrade"
                    );
                }
            }
        }

        crate::core::websocket::WsConnection::connect_h1(
            &self.connector,
            url,
            proxy,
            &self.user_agent,
            &origin,
            extra_headers,
        )
        .await
    }

    /// Force the HTTP/1.1 Upgrade WebSocket path, skipping the
    /// HTTP/2 extended CONNECT probe. Useful against servers that
    /// speak both protocols but whose H2 WebSocket implementation is
    /// known-broken, or for deterministic test setups.
    pub async fn websocket_http1(&self, url: &str) -> Result<crate::core::websocket::WsConnection> {
        self.websocket_with_options(url, self.websocket_config, true, None, &[])
            .await
    }
}

/// Builder returned by [`Session::websocket_builder`].
pub struct WebSocketBuilder<'a> {
    session: &'a Session,
    url: &'a str,
    config: WebSocketConfig,
    force_http1: bool,
    proxy: Option<String>,
    headers: Vec<(String, String)>,
}

impl<'a> WebSocketBuilder<'a> {
    /// Replace WebSocket limits/preferences for this connection.
    pub fn config(mut self, config: WebSocketConfig) -> Self {
        self.config = config;
        self
    }

    /// Force the HTTP/1.1 Upgrade path.
    pub fn http1(mut self) -> Self {
        self.force_http1 = true;
        self
    }

    /// Override the session proxy for this WebSocket connection.
    pub fn proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.proxy = Some(proxy_url.into());
        self
    }

    /// Forward these request headers on the WebSocket handshake — e.g. `Cookie`,
    /// `Authorization`, `Sec-WebSocket-Protocol`, or a captured browser's
    /// `User-Agent`/`Origin`. Handshake-control headers (`Host`, `Connection`,
    /// `Upgrade`, `Sec-WebSocket-Key`/`-Version`/`-Extensions`) are ignored so a
    /// forwarded copy can't break the upgrade. Replaces the current list.
    pub fn headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.headers = headers.into_iter().collect();
        self
    }

    /// Add a single header to forward on the handshake (see [`headers`](Self::headers)).
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Connect.
    pub async fn connect(self) -> Result<crate::core::websocket::WsConnection> {
        self.session
            .websocket_with_options(
                self.url,
                self.config,
                self.force_http1,
                self.proxy.as_deref(),
                &self.headers,
            )
            .await
    }
}

/// Build a WebSocket `Origin` header value from a `ws://` or `wss://`
/// URL by mapping the scheme to `http`/`https`. Used by the WebSocket
/// entry points to keep Origin consistent between H1 and H2 paths.
fn ws_origin(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url)?;
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => "http",
        other => other,
    };
    Ok(format!("{}://{}", scheme, parsed.host_str().unwrap_or("")))
}
