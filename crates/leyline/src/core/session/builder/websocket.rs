use super::SessionBuilder;

impl SessionBuilder {
    /// Sets whether WebSockets try HTTP/2 extended CONNECT before the HTTP/1.1 upgrade.
    pub fn websocket_http2(mut self, on: bool) -> Self {
        self.websocket_config.prefer_http2 = on;
        self
    }

    /// Sets the largest WebSocket frame the session accepts, or no limit for `None`.
    pub fn websocket_max_frame_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.websocket_config.max_frame_size = n.into();
        self
    }

    /// Sets the largest WebSocket message the session accepts, or no limit for `None`.
    pub fn websocket_max_message_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.websocket_config.max_message_size = n.into();
        self
    }

    /// Sets the WebSocket read buffer size, or the default for `None`.
    pub fn websocket_read_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.websocket_config.read_buffer_size = n.into();
        self
    }

    /// Sets the WebSocket write buffer size, or the default for `None`.
    pub fn websocket_write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.websocket_config.write_buffer_size = n.into();
        self
    }

    /// Sets the largest WebSocket write buffer, or the default for `None`.
    pub fn websocket_max_write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.websocket_config.max_write_buffer_size = n.into();
        self
    }

    /// Sets whether WebSockets accept unmasked frames from the server.
    pub fn websocket_accept_unmasked_frames(mut self, on: bool) -> Self {
        self.websocket_config.accept_unmasked_frames = on;
        self
    }
}
