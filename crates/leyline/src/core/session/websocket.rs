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
            let _pass = self.admit_host(&http_url).await?;
            let conn = self
                .websocket_with_options(&http_url, headers, carry)
                .await?;
            self.store_cookies(&conn.header_map(), &http_url);
            Ok::<_, Error>(conn)
        };
        self.unless_shut_down(self.deadline(carry.timeouts.as_ref()).total(handshake))
            .await
    }

    async fn websocket_with_options(
        &self,
        http_url: &url::Url,
        extra_headers: &HeaderList,
        carry: &Carry,
    ) -> Result<WebSocket> {
        let config = self.inner.inputs.websocket_config;
        let proxy = self.proxy_for(http_url, carry.proxy.as_ref())?;
        let extra_headers = self.websocket_headers(http_url, extra_headers);
        let origin = Origin::ascii(http_url);
        let header_order = carry
            .header_order
            .as_deref()
            .map(Cow::Borrowed)
            .or_else(|| self.session_header_order());
        let handshake = Handshake {
            url: http_url,
            proxy,
            user_agent: &self.inner.user_agent,
            origin: &origin,
            extra_headers: &extra_headers,
            header_order: header_order.as_deref(),
            options: &config,
        };

        if config.prefer_http2 && self.inner.inputs.protocol_policy.tcp_offers_h2() {
            if let Some(conn) = WebSocket::connect_h2(
                &self.inner.pool,
                &self.inner.connector,
                &self.inner.h2_config,
                &handshake,
            )
            .await?
            {
                return Ok(conn);
            }
            tracing::debug!("H2 extended CONNECT not available, falling back to H1 Upgrade");
        }

        WebSocket::connect_h1(&self.inner.connector, &handshake).await
    }

    fn websocket_headers(&self, http_url: &url::Url, caller: &HeaderList) -> Vec<(String, String)> {
        let caller_has = |name: &str| caller.get(name).is_some();
        let mut headers = Vec::new();
        self.merge_session_headers(&mut headers, &caller_has, false, http_url);
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
