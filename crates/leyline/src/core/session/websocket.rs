use super::Session;
use super::header_merge::apply_extra_headers;
use crate::core::config::{Proxy, Timeout};
use crate::core::error::{Cause, Error, Result};
use crate::core::headers::HeaderList;
use crate::core::websocket::WebSocket;
use crate::pool::OriginFlag;
use crate::util::sensitive_header;

impl Session {
    pub(crate) async fn open_websocket(
        &self,
        url: &url::Url,
        proxy: Option<&Proxy>,
        headers: &HeaderList,
        timeout: Option<&Timeout>,
    ) -> Result<WebSocket> {
        let handshake = async {
            let _pass = self.inner.host_limits.admit(&http_equivalent(url)).await;
            self.websocket_with_options(url.as_str(), proxy, headers)
                .await
        };
        self.unless_shut_down(self.deadline(timeout).total(handshake))
            .await
    }

    pub(crate) async fn websocket_with_options(
        &self,
        url: &str,
        request_proxy: Option<&Proxy>,
        extra_headers: &HeaderList,
    ) -> Result<WebSocket> {
        let config = self.inner.websocket_config;
        let origin = ws_origin(url)?;
        let parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        let proxy = self.proxy_for(&parsed, request_proxy)?;
        let extra_headers = &self.websocket_headers(&parsed, extra_headers);

        let h1_only = parsed.host_str().is_some_and(|host| {
            self.inner.pool.recalls(
                OriginFlag::H1Only,
                host,
                parsed.port_or_known_default().unwrap_or(443),
                proxy,
            )
        });

        if config.prefer_http2 && !h1_only {
            match WebSocket::connect_h2(
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
                Err(e) if WebSocket::is_h2_fallback_trigger(&e) => {
                    if crate::core::transport::is_h2_alpn_mismatch(&e)
                        && let Some(host) = parsed.host_str()
                    {
                        let port = parsed.port_or_known_default().unwrap_or(443);
                        self.inner
                            .pool
                            .remember(OriginFlag::H1Only, host, port, proxy);
                    }
                    tracing::debug!(
                        error = %e,
                        "H2 extended CONNECT not available, falling back to H1 Upgrade"
                    );
                }
                Err(e) => return Err(e),
            }
        }

        WebSocket::connect_h1(
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

impl Session {
    fn websocket_headers(&self, url: &url::Url, caller: &HeaderList) -> Vec<(String, String)> {
        let caller_has = |name: &str| caller.get(name).is_some();
        let cookie_url = http_equivalent(url);
        let lookup = &cookie_url;
        let mut headers = Vec::new();
        self.merge_session_headers(&mut headers, &caller_has, false, lookup);
        apply_extra_headers(&mut headers, caller, false, &sensitive_header);
        let site = super::execute::SiteContext {
            fetch_site: crate::FetchSite::SameOrigin,
            initiator: Some(lookup),
            chain: &[],
        };
        self.add_jar_cookie(&mut headers, lookup, &site, "GET");
        headers
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.to_string()))
            .collect()
    }
}

fn http_equivalent(url: &url::Url) -> url::Url {
    let mut http = url.clone();
    let scheme = match url.scheme() {
        "ws" => "http",
        _ => "https",
    };
    if http.set_scheme(scheme).is_err() {
        return url.clone();
    }
    http
}

fn ws_origin(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => {
            return Err(Error::new(Cause::Request)
                .with_message("plaintext ws:// is not supported; use wss://"));
        }
        other => {
            return Err(Error::new(Cause::Request)
                .with_message(format!("websocket URL must use wss://, not {other}")));
        }
    };
    let host = parsed.host_str().unwrap_or("");
    Ok(match parsed.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
