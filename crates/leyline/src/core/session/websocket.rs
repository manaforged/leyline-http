use std::fmt;

use super::Session;
use crate::core::error::{Error, Kind, Result};
use crate::core::{IntoParamPair, IntoUrl, ProxyConfig, WebSocketConfig};
use crate::trace::masked;
use crate::util::redact;

impl Session {
    pub fn websocket(&self, url: impl IntoUrl) -> WebSocketBuilder {
        WebSocketBuilder {
            session: self.clone(),
            url: url.into_url(),
            config: self.inner.websocket_config,
            proxy: None,
            headers: Vec::new(),
        }
    }

    async fn websocket_with_options(
        &self,
        url: &str,
        config: WebSocketConfig,
        request_proxy: Option<&ProxyConfig>,
        extra_headers: &[(String, String)],
    ) -> Result<crate::core::websocket::WsConnection> {
        let origin = ws_origin(url)?;
        let parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        let proxy = self.proxy_for(&parsed, request_proxy)?;
        let extra_headers = &self.websocket_headers(&parsed, extra_headers);

        let h1_only = parsed.host_str().is_some_and(|host| {
            self.inner
                .pool
                .is_h1_only(host, parsed.port_or_known_default().unwrap_or(443), proxy)
        });

        if config.prefer_http2 && !h1_only {
            match crate::core::websocket::WsConnection::connect_h2(
                &self.inner.pool,
                &self.inner.connector,
                &self.inner.h2_config,
                url,
                proxy,
                &self.inner.user_agent,
                &origin,
                extra_headers,
                &config,
            )
            .await
            {
                Ok(conn) => return Ok(conn),
                Err(e) if crate::core::websocket::WsConnection::is_h2_fallback_trigger(&e) => {
                    if crate::core::transport::is_h2_alpn_mismatch(&e)
                        && let Some(host) = parsed.host_str()
                    {
                        let port = parsed.port_or_known_default().unwrap_or(443);
                        self.inner.pool.note_h1_only(host, port, proxy);
                    }
                    tracing::debug!(
                        error = %e,
                        "H2 extended CONNECT not available, falling back to H1 Upgrade"
                    );
                }
                Err(e) => return Err(e),
            }
        }

        crate::core::websocket::WsConnection::connect_h1(
            &self.inner.connector,
            url,
            proxy,
            &self.inner.user_agent,
            &origin,
            extra_headers,
            self.session_header_order().as_deref(),
            &config,
        )
        .await
    }
}

#[must_use = "builders are lazy: nothing happens until `.connect()` / await"]
pub struct WebSocketBuilder {
    session: Session,
    url: Result<url::Url>,
    config: WebSocketConfig,
    proxy: Option<ProxyConfig>,
    headers: Vec<(String, String)>,
}

impl fmt::Debug for WebSocketBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let url = self.url.as_ref().map(|url| redact(url.as_str()));
        f.debug_struct("WebSocketBuilder")
            .field("url", &url)
            .field("config", &self.config)
            .field("proxy", &self.proxy)
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
            )
            .finish_non_exhaustive()
    }
}

impl WebSocketBuilder {
    pub fn config(mut self, config: WebSocketConfig) -> Self {
        self.config = config;
        self
    }

    pub fn proxy(mut self, config: impl Into<ProxyConfig>) -> Self {
        self.proxy = Some(config.into());
        self
    }

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

    pub fn header(mut self, name: &str, value: &str) -> Self {
        set_header(&mut self.headers, name.to_string(), value.to_string());
        self
    }

    pub async fn connect(self) -> Result<crate::core::websocket::WsConnection> {
        let url = self.url?;
        let handshake = self.session.websocket_with_options(
            url.as_str(),
            self.config,
            self.proxy.as_ref(),
            &self.headers,
        );
        self.session.deadline(None).total(handshake).await
    }
}

impl std::future::IntoFuture for WebSocketBuilder {
    type Output = Result<crate::core::websocket::WsConnection>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.connect().await })
    }
}

impl Session {
    fn websocket_headers(
        &self,
        url: &url::Url,
        caller: &[(String, String)],
    ) -> Vec<(String, String)> {
        let caller_has = |name: &str| caller.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
        let mut headers = Vec::new();
        self.merge_session_headers(&mut headers, &caller_has, false);
        headers.extend(
            caller
                .iter()
                .map(|(k, v)| (k.clone().into(), v.clone().into())),
        );
        let mut cookie_url = url.clone();
        let lookup = if cookie_url.set_scheme("https").is_ok() {
            &cookie_url
        } else {
            url
        };
        self.add_jar_cookie(&mut headers, lookup, &[], "GET");
        headers
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }
}

fn ws_origin(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => {
            return Err(Error::new(Kind::Request)
                .with_message("plaintext ws:// is not supported; use wss://"));
        }
        other => {
            return Err(Error::new(Kind::Request)
                .with_message(format!("websocket URL must use wss://, not {other}")));
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
