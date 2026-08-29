//! HPACK header compression (RFC 7541).
//!
//! Designed for profile-driven encoding, where the bytes we produce are
//! compared against the selected browser profile's expected behavior.

#![forbid(unsafe_code)]
mod decoder;
mod encoder;
mod huffman;
mod integer;
mod table;

pub use decoder::Decoder;
pub use encoder::Encoder;
pub use table::DynamicTable;
