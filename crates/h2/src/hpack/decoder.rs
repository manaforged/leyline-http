//! HPACK decoder (RFC 7541 Section 6).
//!
//! Decodes header blocks from the wire format back into name-value pairs.

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

/// A decoded header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
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
                headers.push(Header {
                    name: name.to_string(),
                    value: value.to_string(),
                });
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
            // Indexed name.
            let (n, _) = table::lookup(name_index, &self.dynamic)
                .ok_or_else(|| format!("invalid name index {name_index}"))?;
            n.to_string()
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

/// Decode a string literal (RFC 7541 Section 5.2).
fn decode_string(src: &[u8]) -> Result<(String, usize), String> {
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
        let decoded = huffman::decode(raw).map_err(|e| e.to_string())?;
        String::from_utf8(decoded).map_err(|e| e.to_string())?
    } else {
        String::from_utf8(raw.to_vec()).map_err(|e| e.to_string())?
    };

    Ok((value, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hpack::Encoder;

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
