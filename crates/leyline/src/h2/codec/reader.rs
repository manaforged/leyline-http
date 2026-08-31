//! Cancel-safe frame reader.

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::h2::H2Error;
use crate::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader};

use super::DEFAULT_MAX_FRAME_SIZE;

/// Reads HTTP/2 frames from an async reader.
pub struct FrameReader<R> {
    inner: R,
    max_frame_size: u32,
    header_buf: [u8; FRAME_HEADER_LEN],
    header_filled: usize,
    payload_state: Option<PayloadState>,
}

struct PayloadState {
    header: FrameHeader,
    buf: BytesMut,
    filled: usize,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    /// Create a new frame reader.
    pub fn new(reader: R) -> Self {
        Self {
            inner: reader,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            header_buf: [0u8; FRAME_HEADER_LEN],
            header_filled: 0,
            payload_state: None,
        }
    }

    /// Update max frame size (after receiving SETTINGS).
    pub fn set_max_frame_size(&mut self, size: u32) {
        self.max_frame_size = size;
    }

    /// Read the next frame.
    pub async fn next(&mut self) -> Result<Option<Frame>, H2Error> {
        while self.payload_state.is_none() && self.header_filled < FRAME_HEADER_LEN {
            let n = self
                .inner
                .read(&mut self.header_buf[self.header_filled..])
                .await
                .map_err(H2Error::Io)?;
            if n == 0 {
                if self.header_filled == 0 {
                    return Ok(None);
                }
                return Err(H2Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed mid-frame-header",
                )));
            }
            self.header_filled += n;
        }

        if self.payload_state.is_none() {
            let header = FrameHeader::parse(&self.header_buf);
            if header.length > self.max_frame_size {
                self.header_filled = 0;
                return Err(H2Error::FrameTooLarge {
                    size: header.length,
                    max: self.max_frame_size,
                });
            }
            let buf = BytesMut::zeroed(header.length as usize);
            self.payload_state = Some(PayloadState {
                header,
                buf,
                filled: 0,
            });
        }

        let state = self
            .payload_state
            .as_mut()
            .expect("payload state set above");
        while state.filled < state.buf.len() {
            let n = self
                .inner
                .read(&mut state.buf[state.filled..])
                .await
                .map_err(H2Error::Io)?;
            if n == 0 {
                return Err(H2Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed mid-frame-payload",
                )));
            }
            state.filled += n;
        }

        let state = self.payload_state.take().expect("payload state set above");
        self.header_filled = 0;
        Frame::parse(state.header, state.buf.freeze()).map(Some)
    }

    /// Get a mutable reference to the inner reader.
    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}
