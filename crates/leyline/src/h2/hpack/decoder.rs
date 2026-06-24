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
mod tests {
    use super::*;
    use crate::h2::hpack::Encoder;

    #[test]
    fn decode_indexed() {
        let mut dec = Decoder::new();
        // 0x82 = indexed, index 2 = :method GET
        let headers = dec.decode_header_block(&[0x82]).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name, ":method");
        assert_eq!(headers[0].value, "GET");
    }

    #[test]
    fn non_utf8_literal_preserves_raw_bytes_for_table_accounting() {
        // A raw string literal carrying non-UTF-8 obs-text must decode to the
        // ORIGINAL bytes (no U+FFFD substitution here): the dynamic table sizes
        // entries by true octet length, so any in-decoder coercion would diverge
        // our table from the peer's. The lossy &str view happens later, at the
        // HeaderStr boundary.
        let (value, consumed) = decode_string(&[0x02, 0xff, 0xfe]).expect("decodes, not error");
        assert_eq!(consumed, 3);
        assert_eq!(
            value.as_ref(),
            &[0xff, 0xfe],
            "raw wire bytes preserved verbatim"
        );
    }

    #[test]
    fn non_utf8_indexed_value_keeps_table_in_lockstep_with_peer() {
        // Eviction-boundary check for the lossy-decode fix: a non-UTF-8 value
        // inserted with incremental indexing must size the dynamic table by its
        // RAW octet length (2 bytes here), not a U+FFFD-expanded length (which
        // would be 6 bytes for two replacement chars) — otherwise our table
        // evicts on a different schedule than the encoder's and later indices
        // resolve wrong. Encode a literal-with-incremental-indexing field whose
        // value is the 2 raw octets 0xFF 0xFE under a literal name.
        let mut dec = Decoder::new();
        // 0x40 = literal w/ incremental indexing, name index 0 (literal name).
        // name: len 1 "x"; value: len 2, raw 0xFF 0xFE.
        let block = [0x40, 0x01, b'x', 0x02, 0xff, 0xfe];
        let headers = dec.decode_header_block(&block).expect("decodes");
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers[0].value.as_ref(),
            &[0xff, 0xfe],
            "raw value preserved"
        );
        // Entry size = name.len(1) + value.len(2) + 32 = 35 — computed from the
        // raw octets, matching what the peer's encoder accounted for.
        assert_eq!(dec.dynamic.size(), 1 + 2 + 32);
        // And the indexed entry round-trips back to the same raw bytes. The
        // newest dynamic entry is HPACK index `STATIC_TABLE.len()`.
        let (n, v) = table::lookup(table::STATIC_TABLE.len(), &dec.dynamic).expect("indexed");
        assert_eq!(n.as_ref(), b"x");
        assert_eq!(v.as_ref(), &[0xff, 0xfe]);
    }

    #[test]
    fn decode_multiple_indexed() {
        let mut dec = Decoder::new();
        // :method GET, :path /, :scheme https
        let headers = dec.decode_header_block(&[0x82, 0x84, 0x87]).unwrap();
        assert_eq!(headers.len(), 3);
        assert_eq!(
            headers[0],
            Header {
                name: ":method".into(),
                value: "GET".into()
            }
        );
        assert_eq!(
            headers[1],
            Header {
                name: ":path".into(),
                value: "/".into()
            }
        );
        assert_eq!(
            headers[2],
            Header {
                name: ":scheme".into(),
                value: "https".into()
            }
        );
    }

    #[test]
    fn encoder_decoder_roundtrip() {
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();

        let original = vec![
            (":method", "GET"),
            (":path", "/api/all"),
            (":scheme", "https"),
            (":authority", "example.com"),
            ("user-agent", "Mozilla/5.0"),
            ("accept", "text/html"),
        ];

        let encoded = enc.encode_header_block(&original);
        let decoded = dec.decode_header_block(&encoded).unwrap();

        assert_eq!(decoded.len(), original.len());
        for (i, &(name, value)) in original.iter().enumerate() {
            assert_eq!(decoded[i].name, name, "header {i} name mismatch");
            assert_eq!(decoded[i].value, value, "header {i} value mismatch");
        }
    }

    #[test]
    fn dynamic_table_roundtrip() {
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();

        // First request: custom header gets added to dynamic table.
        let block1 = enc.encode_header_block(&[("x-request-id", "abc123")]);
        let headers1 = dec.decode_header_block(&block1).unwrap();
        assert_eq!(headers1[0].name, "x-request-id");
        assert_eq!(headers1[0].value, "abc123");

        // Second request: same header name, different value.
        // Encoder should use indexed name from dynamic table.
        let block2 = enc.encode_header_block(&[("x-request-id", "def456")]);
        let headers2 = dec.decode_header_block(&block2).unwrap();
        assert_eq!(headers2[0].name, "x-request-id");
        assert_eq!(headers2[0].value, "def456");

        // The second block should be shorter (name is indexed).
        assert!(
            block2.len() < block1.len(),
            "second block should be shorter due to dynamic table"
        );
    }

    #[test]
    fn table_size_update() {
        let mut enc = Encoder::new();
        let mut dec = Decoder::new();

        enc.set_max_table_size(0); // clear dynamic table
        let block = enc.encode_header_block(&[(":method", "GET")]);

        // Should start with a size update, then indexed header.
        let headers = dec.decode_header_block(&block).unwrap();
        assert_eq!(headers[0].name, ":method");
        assert_eq!(headers[0].value, "GET");
    }

    #[test]
    fn invalid_index_rejected() {
        let mut dec = Decoder::new();
        // Index 200 doesn't exist in static or dynamic table.
        // 0x80 | prefix_max = 0xFF means index >= 127, then continuation byte.
        let result = dec.decode_header_block(&[0xFF, 0x49]); // index = 127 + 73 = 200
        assert!(result.is_err());
    }
}
