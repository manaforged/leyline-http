use std::borrow::Cow;

use super::Session;
use super::header_merge::apply_extra_headers;
use crate::core::config::Origin;
use crate::core::headers::HeaderList;
use crate::core::request::Carry;
use crate::core::websocket::{Handshake, WebSocket, http_equivalent};
use crate::error::{Error, Result};

impl Session {
    pub(crate) async fn open_websocket(
        &self,
        url: &url::Url,
        headers: &HeaderList,
        carry: &Carry,
    ) -> Result<WebSocket> {
        let handshake = async {
            let http_url = http_equivalent(url)?;
            self.check_https_only(&http_url)?;
            let _pass = self.admit_host(&http_url).await?;
            let conn = self
                .websocket_with_options(&http_url, headers, carry)
                .await?;
            Ok::<_, Error>(conn)
        };
        self.unless_shut_down(self.deadline(carry.timeouts()).total(handshake))
            .await
    }

    async fn websocket_with_options(
        &self,
        http_url: &url::Url,
        extra_headers: &HeaderList,
        carry: &Carry,
    ) -> Result<WebSocket> {
        let config = self.inner.inputs.websocket_config;
        let protocol = carry.protocol.unwrap_or(self.inner.inputs.protocol_policy);
        #[cfg(feature = "http3")]
        if matches!(protocol, super::Protocol::Http3) {
            return Err(Error::config(
                "WebSocket handshakes are not supported over HTTP/3",
            ));
        }
        let (lease_origin, lease, leased_session) = match self
            .proxy_pool()
            .filter(|_| carry.proxy().is_none())
            .map(|pool| pool.acquire_session(self, http_url, None))
            .transpose()?
            .flatten()
        {
            Some((origin, lease, session)) => (origin, Some(lease), Some(session)),
            None => (None, None, None),
        };
        let session = leased_session.as_ref().unwrap_or(self);
        let request_proxy = lease
            .as_ref()
            .map(crate::core::proxy_pool::Lease::config)
            .or_else(|| carry.proxy());
        let proxy = session.proxy_for(http_url, request_proxy)?;
        let extra_headers = session.websocket_headers(http_url, extra_headers);
        let origin = Origin::ascii(http_url);
        let header_order = carry
            .header_order()
            .map(Cow::Borrowed)
            .or_else(|| session.session_header_order());
        let handshake = Handshake {
            url: http_url,
            proxy: proxy.as_deref(),
            user_agent: &session.inner.user_agent,
            origin: &origin,
            extra_headers: &extra_headers,
            header_order: header_order.as_deref(),
            options: &config,
        };

        let result = async {
            let try_h2 = matches!(protocol, super::Protocol::Http2)
                || (config.prefer_http2 && protocol.tcp_offers_h2());
            if try_h2 {
                if let Some(conn) = WebSocket::connect_h2(
                    &session.inner.pool,
                    &session.inner.connector,
                    &session.inner.h2_config,
                    &handshake,
                )
                .await?
                {
                    return Ok(conn);
                }
                if matches!(protocol, super::Protocol::Http2) {
                    return Err(Error::new(crate::error::Kind::Http2)
                        .with_message("HTTP/2 extended CONNECT is not offered by the server"));
                }
                tracing::debug!("H2 extended CONNECT not available, falling back to H1 Upgrade");
            }

            WebSocket::connect_h1(&session.inner.connector, &handshake).await
        }
        .await;
        if proxy.is_some()
            && let Some(lease) = &lease
        {
            lease.record_handshake(&result, lease_origin.as_ref());
        }
        if let Ok(conn) = &result {
            session.store_cookies(conn.headers(), http_url);
        }
        result
    }

    fn websocket_headers(&self, http_url: &url::Url, caller: &HeaderList) -> Vec<(String, String)> {
        let mut headers = Vec::new();
        self.merge_session_headers(&mut headers, Some(caller), false, http_url);
        apply_extra_headers(&mut headers, caller, false);
        let site = super::execute::SiteContext {
            fetch_site: crate::core::FetchSite::SameOrigin,
            initiator: Some(http_url),
            chain: &[],
        };
        self.add_jar_cookie(&mut headers, http_url, &site, &http::Method::GET);
        headers
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v.to_string()))
            .collect()
    }
}

#[cfg(test)]
#[path = "websocket_tests.rs"]
mod tests;
