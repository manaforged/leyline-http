use bytes::Bytes;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable, Typed};
use crate::h2::frame::DEFAULT_HEADER_TABLE_SIZE;

type Indexed<'a> = (&'a [u8], &'a [u8], &'a Typed, usize);

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

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            dynamic: DynamicTable::new(),
            max_table_size: DEFAULT_HEADER_TABLE_SIZE as usize,
            max_header_list_size: crate::wire::DEFAULT_MAX_HEADER_LIST_BYTES,
        }
    }

    pub fn set_max_table_size(&mut self, size: usize) {
        self.max_table_size = size;
        self.dynamic.set_max_size(size);
    }

    pub fn set_max_header_list_size(&mut self, size: usize) {
        self.max_header_list_size = size;
    }

    pub(crate) fn max_header_list_size(&self) -> usize {
        self.max_header_list_size
    }

    #[cfg(any(test, feature = "bench-internals"))]
    pub fn decode_header_block(&mut self, src: &[u8]) -> Result<Vec<Header>, String> {
        let mut headers = Vec::new();
        self.decode_block(src, |name, value, _| {
            headers.push(Header {
                name: Bytes::copy_from_slice(name),
                value: Bytes::copy_from_slice(value),
            });
        })?;
        Ok(headers)
    }

    pub fn decode_block(
        &mut self,
        src: &[u8],
        mut each: impl FnMut(&[u8], &[u8], Typed),
    ) -> Result<(), String> {
        let mut total_size = 0usize;
        let mut pos = 0;
        let mut size_updates_left = MAX_TABLE_SIZE_UPDATES;

        while pos < src.len() {
            let byte = src[pos];
            if byte & 0xE0 == 0x20 {
                pos += self.decode_table_size_update(src, pos, &mut size_updates_left)?;
                continue;
            }
            size_updates_left = 0;
            if byte & 0x80 != 0 {
                let (name, value, typed, consumed) = self.decode_indexed(src, pos)?;
                pos += consumed;
                self.account(&mut total_size, name.len() + value.len())?;
                each(name, value, typed.clone());
                continue;
            }
            let (prefix, mask, add_to_table) = match byte {
                b if b & 0xC0 == 0x40 => (6, 0x3F, true),
                b if b & 0xE0 == 0x00 => (4, 0x0F, false),
                _ => return Err(format!("unexpected byte {byte:#04x} at position {pos}")),
            };
            let (header, typed, consumed) =
                self.decode_literal(src, pos, prefix, mask, add_to_table)?;
            pos += consumed;
            self.account(&mut total_size, header.name.len() + header.value.len())?;
            each(&header.name, &header.value, typed);
        }
        Ok(())
    }

    #[inline]
    fn decode_indexed(&self, src: &[u8], pos: usize) -> Result<Indexed<'_>, String> {
        let (index, consumed) =
            integer::decode(src[pos], 7, &src[pos + 1..]).map_err(|e| e.to_string())?;
        if index == 0 {
            return Err("HPACK index 0 is invalid".into());
        }
        let (name, value, typed) =
            table::entry(index, &self.dynamic).ok_or_else(|| format!("invalid index {index}"))?;
        Ok((name, value, typed, 1 + consumed))
    }

    fn account(&self, total_size: &mut usize, len: usize) -> Result<(), String> {
        *total_size += len + table::ENTRY_OVERHEAD;
        if *total_size > self.max_header_list_size {
            return Err(format!(
                "decoded header list size {} exceeds max {}",
                total_size, self.max_header_list_size
            ));
        }
        Ok(())
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
            integer::decode(src[pos], 5, &src[pos + 1..]).map_err(|e| e.to_string())?;
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
    ) -> Result<(Header, Typed, usize), String> {
        let byte = src[pos];
        let mut offset = pos + 1;

        let (name_index, consumed) =
            integer::decode(byte & mask, prefix_bits, &src[offset..]).map_err(|e| e.to_string())?;
        offset += consumed;

        let (name, typed_name) = if name_index == 0 {
            let (s, consumed) = decode_string(&src[offset..])?;
            offset += consumed;
            let typed = parse_name(&s);
            (s, typed)
        } else {
            let (n, _) = table::lookup(name_index, &self.dynamic)
                .ok_or_else(|| format!("invalid name index {name_index}"))?;
            let typed = match table::entry(name_index, &self.dynamic) {
                Some((_, _, Some((typed, _)))) => Some(typed.clone()),
                _ => parse_name(&n),
            };
            (n, typed)
        };

        let (value, consumed) = decode_string(&src[offset..])?;
        offset += consumed;

        let typed = typed_name.and_then(|name| {
            http::HeaderValue::from_maybe_shared(value.clone())
                .ok()
                .map(|value| (name, value))
        });
        if add_to_table {
            self.dynamic
                .insert_typed(name.clone(), value.clone(), typed.clone());
        }

        Ok((Header { name, value }, typed, offset - pos))
    }
}

fn parse_name(name: &[u8]) -> Option<http::HeaderName> {
    if name.starts_with(b":") {
        return None;
    }
    http::HeaderName::from_bytes(name).ok()
}

fn decode_string(src: &[u8]) -> Result<(Bytes, usize), String> {
    if src.is_empty() {
        return Err("unexpected end of string".into());
    }

    let huffman_encoded = src[0] & 0x80 != 0;
    let (length, consumed) = integer::decode(src[0], 7, &src[1..]).map_err(|e| e.to_string())?;
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
