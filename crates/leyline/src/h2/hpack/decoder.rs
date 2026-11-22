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

#[cfg(any(test, feature = "bench-internals"))]
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
            let consumed = self.decode_literal(
                src,
                pos,
                prefix,
                mask,
                add_to_table,
                &mut total_size,
                &mut each,
            )?;
            pos += consumed;
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

    #[expect(
        clippy::too_many_arguments,
        reason = "internal literal header decode passes parser cursor and callback"
    )]
    fn decode_literal(
        &mut self,
        src: &[u8],
        pos: usize,
        prefix_bits: u8,
        mask: u8,
        add_to_table: bool,
        total_size: &mut usize,
        each: &mut impl FnMut(&[u8], &[u8], Typed),
    ) -> Result<usize, String> {
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
            let (n, _, entry_typed) = table::entry(name_index, &self.dynamic)
                .ok_or_else(|| format!("invalid name index {name_index}"))?;
            let typed = match entry_typed {
                Some((typed, _)) => Some(typed.clone()),
                None => parse_name(n),
            };
            (std::borrow::Cow::Borrowed(n), typed)
        };

        let (value, consumed) = decode_string(&src[offset..])?;
        offset += consumed;

        self.account(total_size, name.len() + value.len())?;
        if !add_to_table {
            let typed = typed_name.and_then(|name| {
                http::HeaderValue::from_bytes(&value)
                    .ok()
                    .map(|value| (name, value))
            });
            each(&name, &value, typed);
            return Ok(offset - pos);
        }

        let name = cow_to_bytes(name);
        let value = cow_to_bytes(value);
        let typed = typed_name.and_then(|name| {
            http::HeaderValue::from_maybe_shared(value.clone())
                .ok()
                .map(|value| (name, value))
        });
        self.dynamic
            .insert_typed(name.clone(), value.clone(), typed.clone());
        each(&name, &value, typed);

        Ok(offset - pos)
    }
}

fn parse_name(name: &[u8]) -> Option<http::HeaderName> {
    if name.starts_with(b":") {
        return None;
    }
    http::HeaderName::from_bytes(name).ok()
}

fn decode_string(src: &[u8]) -> Result<(std::borrow::Cow<'_, [u8]>, usize), String> {
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
        std::borrow::Cow::Owned(huffman::decode(raw).map_err(|e| e.to_string())?)
    } else {
        std::borrow::Cow::Borrowed(raw)
    };

    Ok((value, end))
}

fn cow_to_bytes(value: std::borrow::Cow<'_, [u8]>) -> Bytes {
    match value {
        std::borrow::Cow::Borrowed(raw) => Bytes::copy_from_slice(raw),
        std::borrow::Cow::Owned(vec) => Bytes::from(vec),
    }
}

#[cfg(test)]
mod tests;
