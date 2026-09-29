use bytes::Bytes;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable};

const MAX_HEADER_LIST_SIZE: usize = 64 * 1024;
const MAX_TABLE_SIZE_UPDATES: u8 = 2;

pub struct Decoder {
    dynamic: DynamicTable,
    max_table_size: usize,
    max_header_list_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: Bytes,
    pub value: Bytes,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            dynamic: DynamicTable::new(),
            max_table_size: 4096,
            max_header_list_size: MAX_HEADER_LIST_SIZE,
        }
    }

    pub fn set_max_table_size(&mut self, size: usize) {
        self.max_table_size = size;
        self.dynamic.set_max_size(size);
    }

    pub fn set_max_header_list_size(&mut self, size: usize) {
        self.max_header_list_size = size;
    }

    pub fn decode_header_block(&mut self, src: &[u8]) -> Result<Vec<Header>, String> {
        let mut headers = Vec::new();
        let mut total_size = 0usize;
        let mut pos = 0;
        let mut size_updates_left = MAX_TABLE_SIZE_UPDATES;

        while pos < src.len() {
            let byte = src[pos];

            let decoded = if byte & 0x80 != 0 {
                let (index, consumed) =
                    integer::decode(byte, 7, &src[pos + 1..], 0).map_err(|e| e.to_string())?;
                pos += 1 + consumed;

                if index == 0 {
                    return Err("HPACK index 0 is invalid".into());
                }

                let (name, value) = table::lookup(index, &self.dynamic)
                    .ok_or_else(|| format!("invalid index {index}"))?;
                Some(Header { name, value })
            } else if byte & 0xC0 == 0x40 {
                let (header, consumed) = self.decode_literal(src, pos, 6, 0x3F, true)?;
                pos += consumed;
                Some(header)
            } else if byte & 0xE0 == 0x00 {
                let (header, consumed) = self.decode_literal(src, pos, 4, 0x0F, false)?;
                pos += consumed;
                Some(header)
            } else if byte & 0xE0 == 0x20 {
                pos += self.decode_table_size_update(src, pos, &mut size_updates_left)?;
                None
            } else {
                return Err(format!("unexpected byte {byte:#04x} at position {pos}"));
            };

            if let Some(header) = decoded {
                size_updates_left = 0;
                total_size += header.name.len() + header.value.len() + 32;
                if total_size > self.max_header_list_size {
                    return Err(format!(
                        "decoded header list size {} exceeds max {}",
                        total_size, self.max_header_list_size
                    ));
                }
                headers.push(header);
            }
        }

        Ok(headers)
    }

    fn decode_table_size_update(
        &mut self,
        src: &[u8],
        pos: usize,
        updates_left: &mut u8,
    ) -> Result<usize, String> {
        if *updates_left == 0 {
            return Err(format!(
                "dynamic table size update at position {pos} not allowed (RFC 7541 §4.2)"
            ));
        }
        *updates_left -= 1;
        let (new_size, consumed) =
            integer::decode(src[pos], 5, &src[pos + 1..], 0).map_err(|e| e.to_string())?;
        if new_size > self.max_table_size {
            return Err(format!(
                "dynamic table size update {new_size} exceeds max {}",
                self.max_table_size
            ));
        }
        self.dynamic.set_max_size(new_size);
        Ok(1 + consumed)
    }

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

        let (name_index, consumed) = integer::decode(byte & mask, prefix_bits, &src[offset..], 0)
            .map_err(|e| e.to_string())?;
        offset += consumed;

        let name = if name_index == 0 {
            let (s, consumed) = decode_string(&src[offset..])?;
            offset += consumed;
            s
        } else {
            let (n, _) = table::lookup(name_index, &self.dynamic)
                .ok_or_else(|| format!("invalid name index {name_index}"))?;
            n
        };

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
