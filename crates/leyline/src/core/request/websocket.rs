use super::RequestBuilder;
use crate::core::error::{Error, Result};
use crate::core::websocket::WebSocket;

impl RequestBuilder {
    /// Opens a WebSocket to the request URL with the request headers, proxy, and timeout.
    pub async fn websocket(self) -> Result<WebSocket> {
        if let Some(err) = self.builder_error {
            return Err(err);
        }
        let Some(mut url) = self.url else {
            return Err(Error::from_url_parse(url::ParseError::EmptyHost));
        };
        if !self.query_params.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query_params);
        }
        self.session
            .open_websocket(
                &url,
                self.proxy.as_ref(),
                &self.headers,
                self.timeouts.as_ref(),
            )
            .await
    }
}
