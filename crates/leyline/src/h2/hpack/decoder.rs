//! HPACK decoder (RFC 7541 Section 6).
//!
//! Decodes header blocks from the wire format back into name-value pairs.

use bytes::Bytes;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable};

/// Max total decoded header size (64KB default, matches Chrome).
const MAX_HEADER_LIST_SIZE: usize = 64 * 1024;

/// HPACK decoder with dynamic table state.
pub struct Decoder {
    dynamic: DynamicTable,
    max_table_size: usize,
    max_header_list_size: usize,
}

/// A decoded header. Both parts are the **original wire `Bytes`**: an indexed
/// header (static or dynamic table hit) materializes by refcount/`from_static`
/// with no heap copy; only a literal value allocates. Bytes are kept verbatim —
/// no UTF-8 coercion here — so the dynamic table sizes entries by their true
/// octet length and stays in lockstep with the peer's table. The lossy `&str`
/// view is materialized later at the `HeaderStr` boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: Bytes,
    pub value: Bytes,
}

impl Decoder {
    /// Create a new decoder with default table size (4096).
    pub fn new() -> Self {
        Self {
            dynamic: DynamicTable::new(),
            max_table_size: 4096,
            max_header_list_size: MAX_HEADER_LIST_SIZE,
        }
    }

    /// Set max table size (from SETTINGS).
    pub fn set_max_table_size(&mut self, size: usize) {
        self.max_table_size = size;
        self.dynamic.set_max_size(size);
    }

    /// Set max decoded header list size.
    pub fn set_max_header_list_size(&mut self, size: usize) {
        self.max_header_list_size = size;
    }

    /// Decode a header block into a list of headers.
    pub fn decode_header_block(&mut self, src: &[u8]) -> Result<Vec<Header>, String> {
        let mut headers = Vec::new();
        let mut total_size = 0usize;
        let mut pos = 0;

        while pos < src.len() {
            let byte = src[pos];

            if byte & 0x80 != 0 {
                // Indexed header field (Section 6.1): 1xxxxxxx
                let (index, consumed) =
                    integer::decode(byte, 7, &src[pos + 1..], 0).map_err(|e| e.to_string())?;
                pos += 1 + consumed;

                // RFC 7541 Section 6.1: index 0 is not used.
                if index == 0 {
                    return Err("HPACK index 0 is invalid".into());
                }

                let (name, value) = table::lookup(index, &self.dynamic)
                    .ok_or_else(|| format!("invalid index {index}"))?;
                headers.push(Header { name, value });
            } else if byte & 0xC0 == 0x40 {
                // Literal with incremental indexing (Section 6.2.1): 01xxxxxx
                let (header, consumed) = self.decode_literal(src, pos, 6, 0x3F, true)?;
                pos += consumed;
                headers.push(header);
            } else if byte & 0xF0 == 0x00 {
                // Literal without indexing (Section 6.2.2): 0000xxxx
                let (header, consumed) = self.decode_literal(src, pos, 4, 0x0F, false)?;
                pos += consumed;
                headers.push(header);
            } else if byte & 0xF0 == 0x10 {
                // Literal never indexed (Section 6.2.3): 0001xxxx
                let (header, consumed) = self.decode_literal(src, pos, 4, 0x0F, false)?;
                pos += consumed;
                headers.push(header);
            } else if byte & 0xE0 == 0x20 {
                // Dynamic table size update (Section 6.3): 001xxxxx
                let (new_size, consumed) =
                    integer::decode(byte, 5, &src[pos + 1..], 0).map_err(|e| e.to_string())?;
                pos += 1 + consumed;

                if new_size > self.max_table_size {
                    return Err(format!(
                        "dynamic table size update {new_size} exceeds max {}",
                        self.max_table_size
                    ));
                }
                self.dynamic.set_max_size(new_size);
            } else {
                return Err(format!("unexpected byte {byte:#04x} at position {pos}"));
            }

            // Check decoded size limit (HPACK bomb protection).
            if let Some(last) = headers.last() {
                total_size += last.name.len() + last.value.len() + 32;
                if total_size > self.max_header_list_size {
                    return Err(format!(
                        "decoded header list size {} exceeds max {}",
                        total_size, self.max_header_list_size
                    ));
                }
            }
        }

        Ok(headers)
    }

    /// Decode a literal header field.
    fn decode_literal(
        &mut self,
        src: &[u8],
        pos: usize,
        prefix_bits: u8,
        mask: u8,
        add_to_table: bool,
    ) -> Result<(Header, usize), String> {
        let byte = src[pos];
        let mut offset = pos + 1;

        // Decode name index or literal name.
        let (name_index, consumed) = integer::decode(byte & mask, prefix_bits, &src[offset..], 0)
            .map_err(|e| e.to_string())?;
        offset += consumed;

        let name = if name_index == 0 {
            // Literal name.
            let (s, consumed) = decode_string(&src[offset..])?;
            offset += consumed;
            s
        } else {
            // Indexed name (static or dynamic table) — refcount/from_static.
            let (n, _) = table::lookup(name_index, &self.dynamic)
                .ok_or_else(|| format!("invalid name index {name_index}"))?;
            n
        };

        // Decode value (always literal).
        let (value, consumed) = decode_string(&src[offset..])?;
        offset += consumed;

        if add_to_table {
            self.dynamic.insert(name.clone(), value.clone());
        }

        Ok((Header { name, value }, offset - pos))
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode a string literal (RFC 7541 Section 5.2) into the **original wire
/// `Bytes`**. A raw literal is copied verbatim; a Huffman literal decodes into a
/// `Vec`. No UTF-8 coercion happens here: the bytes feed the dynamic table at
/// their true octet length so eviction stays in lockstep with the peer. Any
/// non-UTF-8 obs-text is coerced lossily (U+FFFD) only later, at the `HeaderStr`
/// boundary ([`crate::core::HeaderStr::from_bytes_lossy`]).
fn decode_string(src: &[u8]) -> Result<(Bytes, usize), String> {
    if src.is_empty() {
        return Err("unexpected end of string".into());
    }

    let huffman_encoded = src[0] & 0x80 != 0;
    let (length, consumed) = integer::decode(src[0], 7, &src[1..], 0).map_err(|e| e.to_string())?;
    let start = 1 + consumed;
    let end = start + length;

    if end > src.len() {
        return Err(format!("string length {length} exceeds remaining data"));
    }

    let raw = &src[start..end];

    let value = if huffman_encoded {
        Bytes::from(huffman::decode(raw).map_err(|e| e.to_string())?)
    } else {
        Bytes::copy_from_slice(raw)
    };

    Ok((value, end))
}

#[cfg(test)]
mod tests;
