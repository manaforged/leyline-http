use super::Session;
use crate::core::error::Result;

impl Session {
    // Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬ WebSocket Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬

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
        let origin = ws_origin(url)?;

        // Try H2 first. The pool helper handles the TLS handshake +
        // ALPN check; if the connection already existed we just clone
        // its handle. Failures that look like "peer didn't enable
        // CONNECT protocol" fall through to the H1 upgrade path.
        match crate::core::websocket::WsConnection::connect_h2(
            &self.pool,
            &self.connector,
            &self.h2_config,
            url,
            self.proxy.as_deref(),
            &self.user_agent,
            &origin,
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

        self.websocket_http1(url).await
    }

    /// Force the HTTP/1.1 Upgrade WebSocket path, skipping the
    /// HTTP/2 extended CONNECT probe. Useful against servers that
    /// speak both protocols but whose H2 WebSocket implementation is
    /// known-broken, or for deterministic test setups.
    pub async fn websocket_http1(&self, url: &str) -> Result<crate::core::websocket::WsConnection> {
        let origin = ws_origin(url)?;
        crate::core::websocket::WsConnection::connect_h1(
            &self.connector,
            url,
            self.proxy.as_deref(),
            &self.user_agent,
            &origin,
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
