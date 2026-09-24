#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebSocketConfig {
    pub(crate) prefer_http2: bool,
    pub(crate) max_frame_size: Option<usize>,
    pub(crate) max_message_size: Option<usize>,
    pub(crate) read_buffer_size: Option<usize>,
    pub(crate) write_buffer_size: Option<usize>,
    pub(crate) max_write_buffer_size: Option<usize>,
    pub(crate) accept_unmasked_frames: bool,
}

impl Default for WebSocketConfig {
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

impl WebSocketConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prefer_http2(mut self, on: bool) -> Self {
        self.prefer_http2 = on;
        self
    }

    pub fn max_frame_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_frame_size = n.into();
        self
    }

    pub fn max_message_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_message_size = n.into();
        self
    }

    pub fn read_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.read_buffer_size = n.into();
        self
    }

    pub fn write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.write_buffer_size = n.into();
        self
    }

    pub fn max_write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_write_buffer_size = n.into();
        self
    }

    pub fn accept_unmasked_frames(mut self, on: bool) -> Self {
        self.accept_unmasked_frames = on;
        self
    }
}
