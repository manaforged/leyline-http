//! HPACK header compression (RFC 7541).

#![forbid(unsafe_code)]
mod decoder;
mod encoder;
mod huffman;
mod integer;
mod table;

pub use decoder::Decoder;
pub use encoder::Encoder;
pub use table::DynamicTable;
