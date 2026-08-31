use super::Session;
use crate::core::IntoParamPair;
use crate::core::WebSocketConfig;
use crate::core::error::{Error, Result};

impl Session {
    /// Start a WebSocket connect.
    pub fn websocket(&self, url: &str) -> WebSocketBuilder {
        WebSocketBuilder {
            session: self.clone(),
            url: url.to_string(),
            config: self.inner.websocket_config.clone(),
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
        let proxy = self.inner.proxy_config.proxy_for(
            &parsed,
            request_proxy,
            self.inner.proxy.as_deref(),
            self.inner.proxy_from_env,
        );

        if config.prefer_http2 && !force_http1 {
            match crate::core::websocket::WsConnection::connect_h2(
                &self.inner.pool,
                &self.inner.connector,
                &self.inner.h2_config,
                url,
                proxy,
                &self.inner.user_agent,
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
            &self.inner.connector,
            url,
            proxy,
            &self.inner.user_agent,
            &origin,
            extra_headers,
        )
        .await
    }
}

/// Builder returned by [`Session::websocket`].
#[must_use = "builders are lazy: nothing happens until `.connect()` / await"]
pub struct WebSocketBuilder {
    session: Session,
    url: String,
    config: WebSocketConfig,
    force_http1: bool,
    proxy: Option<String>,
    headers: Vec<(String, String)>,
}

impl WebSocketBuilder {
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

    /// Set handshake headers. Same-name values replace, like [`crate::RequestBuilder::headers`].
    pub fn headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (name, value) = pair.into_param_pair();
            set_header(&mut self.headers, name, value);
        }
        self
    }

    /// Set one handshake header (see [`headers`](Self::headers)).
    pub fn header(mut self, name: &str, value: &str) -> Self {
        set_header(&mut self.headers, name.to_string(), value.to_string());
        self
    }

    /// Connect.
    pub async fn connect(self) -> Result<crate::core::websocket::WsConnection> {
        self.session
            .websocket_with_options(
                &self.url,
                self.config,
                self.force_http1,
                self.proxy.as_deref(),
                &self.headers,
            )
            .await
    }
}

impl std::future::IntoFuture for WebSocketBuilder {
    type Output = Result<crate::core::websocket::WsConnection>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.connect().await })
    }
}

/// Build a WebSocket `Origin` header value from a `wss://` URL by mapping the scheme to `https`.
fn ws_origin(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url)?;
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => {
            return Err(Error::Http(
                "plaintext ws:// is not supported; use wss://".into(),
            ));
        }
        other => {
            return Err(Error::Http(format!(
                "websocket URL must use wss://, not {other}"
            )));
        }
    };
    let host = parsed.host_str().unwrap_or("");
    Ok(match parsed.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

fn set_header(headers: &mut Vec<(String, String)>, name: String, value: String) {
    if let Some((_, existing)) = headers
        .iter_mut()
        .find(|(key, _)| key.eq_ignore_ascii_case(&name))
    {
        *existing = value;
    } else {
        headers.push((name, value));
    }
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
