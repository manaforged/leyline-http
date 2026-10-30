#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WebSocketOptions {
    pub(crate) prefer_http2: bool,
    pub(crate) max_frame_size: Option<usize>,
    pub(crate) max_message_size: Option<usize>,
    pub(crate) read_buffer_size: Option<usize>,
    pub(crate) write_buffer_size: Option<usize>,
    pub(crate) max_write_buffer_size: Option<usize>,
    pub(crate) accept_unmasked_frames: bool,
}

impl Default for WebSocketOptions {
    fn default() -> Self {
        Self {
            prefer_http2: true,
            max_frame_size: None,
            max_message_size: None,
            read_buffer_size: None,
            write_buffer_size: None,
            max_write_buffer_size: None,
            accept_unmasked_frames: false,
        }
    }
}
