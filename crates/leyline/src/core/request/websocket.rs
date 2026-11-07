use super::RequestBuilder;
use crate::core::error::{Error, Result};
use crate::core::websocket::WebSocket;

impl RequestBuilder {
    /// Opens a WebSocket to the request URL with the request headers, proxy, and timeout.
    pub async fn websocket(self) -> Result<WebSocket> {
        if let Some(err) = self.draft.builder_error {
            return Err(err);
        }
        let Some(url) = self.draft.url else {
            return Err(Error::from(url::ParseError::EmptyHost));
        };
        let mut url = std::sync::Arc::unwrap_or_clone(url);
        if !self.draft.query_params.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.draft.query_params);
        }
        self.session
            .open_websocket(
                &url,
                self.draft.proxy.as_ref(),
                &self.draft.headers,
                self.draft.timeouts.as_ref(),
            )
            .await
    }
}
