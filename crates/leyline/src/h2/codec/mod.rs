//! Frame codec — reads and writes HTTP/2 frames over async IO.
//!
//! No tokio_util dependency. Just a buffer + AsyncRead/AsyncWrite.

#![forbid(unsafe_code)]
mod reader;
mod writer;

pub use reader::FrameReader;
pub use writer::FrameWriter;

/// Default max frame payload size (RFC 9113 Section 4.2).
pub(crate) const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;

#[cfg(test)]
mod tests;
