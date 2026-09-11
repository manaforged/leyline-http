#![forbid(unsafe_code)]
mod reader;
mod writer;

pub use reader::FrameReader;
pub use writer::FrameWriter;

pub(crate) const DEFAULT_MAX_FRAME_SIZE: u32 = 16_384;

#[cfg(test)]
mod tests;
