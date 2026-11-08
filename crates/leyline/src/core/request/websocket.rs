use super::RequestBuilder;
use crate::core::websocket::WebSocket;
use crate::error::Result;

impl RequestBuilder {
    /// Opens a WebSocket to the request URL with the request headers, header order, proxy, and timeout.
    pub async fn websocket(mut self) -> Result<WebSocket> {
        let url = self.resolve()?;
        self.session
            .open_websocket(&url, &self.draft.headers, &self.draft.carry)
            .await
    }
}
