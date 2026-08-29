//! HPACK encoder (RFC 7541 Section 6).
//!
//! Encodes headers into the wire format that matches what real browsers produce.
//! Uses indexed representations where possible, Huffman-encodes values when shorter.

use bytes::Bytes;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable};

/// HPACK encoder with dynamic table state.
pub struct Encoder {
    dynamic: DynamicTable,
    /// Pending table size update to signal at start of next header block.
    pending_size_update: Option<usize>,
}

impl Encoder {
    /// Create a new encoder with default table size (4096).
    pub fn new() -> Self {
        Self {
            dynamic: DynamicTable::new(),
            pending_size_update: None,
        }
    }

    /// Set the max dynamic table size (from SETTINGS_HEADER_TABLE_SIZE).
    /// The size update is signaled at the start of the next header block.
    pub fn set_max_table_size(&mut self, size: usize) {
        self.dynamic.set_max_size(size);
        self.pending_size_update = Some(size);
    }

    /// Encode a header block (list of name-value pairs).
    /// Returns the HPACK-encoded bytes.
    pub fn encode_header_block(&mut self, headers: &[(&str, &str)]) -> Vec<u8> {
        self.encode_header_block_iter(headers.iter().copied(), headers.len())
    }

    /// Encode an ordered sequence of header pairs in one pass, without the
    /// caller first collecting them into a single slice. The request path
    /// uses this to encode pseudo-headers then regular headers with no
    /// intermediate `Vec`; `count` presizes the output so the encode does
    /// not realloc as it grows.
    pub fn encode_header_block_iter<'a>(
        &mut self,
        headers: impl Iterator<Item = (&'a str, &'a str)>,
        count: usize,
    ) -> Vec<u8> {
        // ~32 bytes/header is a generous average for Chrome request headers.
        let mut dst = Vec::with_capacity(count.saturating_mul(32).max(64));

        // Signal any pending table size update (RFC 7541 Section 6.3).
        if let Some(size) = self.pending_size_update.take() {
            integer::encode(size, 5, 0x20, &mut dst);
        }

        for (name, value) in headers {
            self.encode_header(name, value, &mut dst);
        }

        dst
    }

    /// Headers that must use "never indexed" representation (RFC 7541 Section 7.1.3).
    /// Prevents sensitive values from being stored in the dynamic table.
    fn is_sensitive(name: &str) -> bool {
        matches!(
            name,
            "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
        )
    }

    fn encode_header(&mut self, name: &str, value: &str, dst: &mut Vec<u8>) {
        // RFC 7540 §8.1.2: HTTP/2 requires header field names to be lowercase.
        // A request with uppercase names is malformed and MUST be treated as a
        // stream error (§8.1.2.6). Normalise here so callers can pass
        // conventionally-cased names without each site-module having to
        // lowercase vendor-provided header names itself.
        // Pseudo-headers (`:method`, `:path`, …) are already lowercase.
        let lowered;
        let name: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            lowered = name.to_ascii_lowercase();
            &lowered
        } else {
            name
        };

        // Sensitive headers must never be indexed (RFC 7541 Section 7.1.3).
        if Self::is_sensitive(name) {
            // Use "never indexed" with static name if available, else new name.
            if let Some((index, _exact)) = table::find_static(name, value) {
                // Literal never indexed, name referenced by index.
                // Prefix: 0001xxxx (4-bit index prefix).
                integer::encode(index, 4, 0x10, dst);
                encode_string(value, dst);
            } else {
                // Literal never indexed, new name.
                dst.push(0x10);
                encode_string(name, dst);
                encode_string(value, dst);
            }
            return;
        }

        // Try static table first.
        if let Some((index, exact)) = table::find_static(name, value) {
            if exact {
                // Indexed header field (RFC 7541 Section 6.1).
                integer::encode(index, 7, 0x80, dst);
                return;
            }
            // Name match only — literal with indexing, name indexed.
            self.encode_literal_indexed_name(index, value, dst);
            return;
        }

        // Try dynamic table.
        let dyn_offset = table::STATIC_TABLE.len();
        let mut dyn_exact = None;
        let mut dyn_name = None;
        for i in 0..self.dynamic.len() {
            if let Some((n, v)) = self.dynamic.get(i) {
                if n.as_ref() == name.as_bytes() {
                    if v.as_ref() == value.as_bytes() {
                        dyn_exact = Some(dyn_offset + i);
                        break;
                    }
                    if dyn_name.is_none() {
                        dyn_name = Some(dyn_offset + i);
                    }
                }
            }
        }

        if let Some(index) = dyn_exact {
            integer::encode(index, 7, 0x80, dst);
            return;
        }
        if let Some(index) = dyn_name {
            self.encode_literal_indexed_name(index, value, dst);
            return;
        }

        // No match — literal with indexing, new name.
        self.encode_literal_new_name(name, value, dst);
    }

    /// Literal header with incremental indexing, name referenced by index.
    /// Prefix: 01xxxxxx (6-bit index prefix).
    fn encode_literal_indexed_name(&mut self, name_index: usize, value: &str, dst: &mut Vec<u8>) {
        integer::encode(name_index, 6, 0x40, dst);
        encode_string(value, dst);
        // Add to dynamic table.
        let name = self.resolve_name(name_index);
        self.dynamic
            .insert(name, Bytes::copy_from_slice(value.as_bytes()));
    }

    /// Literal header with incremental indexing, new name.
    /// Prefix: 01000000 (index = 0).
    fn encode_literal_new_name(&mut self, name: &str, value: &str, dst: &mut Vec<u8>) {
        dst.push(0x40); // Literal with indexing, name index = 0
        encode_string(name, dst);
        encode_string(value, dst);
        // Add to dynamic table.
        self.dynamic.insert(
            Bytes::copy_from_slice(name.as_bytes()),
            Bytes::copy_from_slice(value.as_bytes()),
        );
    }

    fn resolve_name(&self, index: usize) -> Bytes {
        if index < table::STATIC_TABLE.len() {
            Bytes::from_static(table::STATIC_TABLE[index].0.as_bytes())
        } else {
            let dyn_idx = index - table::STATIC_TABLE.len();
            self.dynamic
                .get(dyn_idx)
                .map(|(n, _)| n.clone())
                .unwrap_or_default()
        }
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode a string value, choosing Huffman if shorter.
fn encode_string(s: &str, dst: &mut Vec<u8>) {
    let huff_len = huffman::encoded_len(s.as_bytes());

    if huff_len < s.len() {
        // Huffman-encoded (bit 7 = 1).
        integer::encode(huff_len, 7, 0x80, dst);
        huffman::encode(s.as_bytes(), dst);
    } else {
        // Raw (bit 7 = 0).
        integer::encode(s.len(), 7, 0x00, dst);
        dst.extend_from_slice(s.as_bytes());
    }
}

#[cfg(test)]
mod tests;
