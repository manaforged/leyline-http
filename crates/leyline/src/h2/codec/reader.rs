//! Cancel-safe frame reader.

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::h2::frame::{Frame, FrameHeader, FRAME_HEADER_LEN};
use crate::h2::H2Error;

use super::DEFAULT_MAX_FRAME_SIZE;

/// Reads HTTP/2 frames from an async reader.
///
/// `next()` is **cancel-safe**: if the returned future is dropped before
/// resolving (e.g. a `tokio::select!` arm lost to a sibling), the partial
/// read state is preserved on the reader and the next `next()` resumes
/// exactly where the cancelled one left off.
///
/// The actor-model driver polls `reader.next()` in a biased `select!`
/// alongside a command channel, body channel, and sweep tick — every one
/// of those sibling completions drops the in-flight read. An earlier
/// implementation used `AsyncReadExt::read_exact`, which is *not*
/// cancel-safe: `read_exact` loops internally and can lose bytes that
/// were already pulled off the socket when the enclosing future is
/// dropped. The result was wire-level desync, surfacing as
/// "frame size <random u24> exceeds max 16384" — the "length" was three
/// arbitrary payload bytes read back as a frame header.
///
/// The fix is to do incremental reads via single `AsyncReadExt::read`
/// calls (which are cancel-safe per Tokio's contract: on Pending no
/// bytes are moved) and persist the cursor between calls.
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

    /// Read the next frame. Returns None on EOF.
    ///
    /// Cancel-safe — see type-level docs.
    pub async fn next(&mut self) -> Result<Option<Frame>, H2Error> {
        // Phase 1: fill the 9-byte frame header if we haven't already.
        while self.payload_state.is_none() && self.header_filled < FRAME_HEADER_LEN {
            let n = self
                .inner
                .read(&mut self.header_buf[self.header_filled..])
                .await
                .map_err(H2Error::Io)?;
            if n == 0 {
                // Clean EOF only at a frame boundary.
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

        // Transition: header fully read → parse + allocate payload buf.
        if self.payload_state.is_none() {
            let header = FrameHeader::parse(&self.header_buf);
            if header.length > self.max_frame_size {
                // Reset so a caller who recovers from this error is at a
                // defined state (though connection-level errors usually
                // terminate the driver anyway).
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

        // Phase 2: fill the payload.
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

        // Consume state and advance to a fresh frame boundary.
        let state = self.payload_state.take().expect("payload state set above");
        self.header_filled = 0;
        Frame::parse(state.header, state.buf.freeze()).map(Some)
    }

    /// Get a mutable reference to the inner reader.
    ///
    /// Callers that bypass `next()` risk desynchronising the internal
    /// cursors; only use this before any `next()` call (e.g. to read the
    /// raw HTTP/2 preface on the server side) or after `next()` has
    /// returned the last frame at a clean boundary.
    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}
