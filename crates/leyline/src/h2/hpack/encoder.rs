use bytes::Bytes;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable};

pub struct Encoder {
    dynamic: DynamicTable,
    pending_size_update: Option<usize>,
}

impl Encoder {
    pub fn new() -> Self {
        Self {
            dynamic: DynamicTable::new(),
            pending_size_update: None,
        }
    }

    pub fn set_max_table_size(&mut self, size: usize) {
        self.dynamic.set_max_size(size);
        self.pending_size_update = Some(size);
    }

    pub fn encode_header_block(&mut self, headers: &[(&str, &str)]) -> Vec<u8> {
        self.encode_header_block_iter(headers.iter().copied(), headers.len())
    }

    pub fn encode_header_block_iter<'a>(
        &mut self,
        headers: impl Iterator<Item = (&'a str, &'a str)>,
        count: usize,
    ) -> Vec<u8> {
        let mut dst = Vec::with_capacity(count.saturating_mul(32).max(64));

        if let Some(size) = self.pending_size_update.take() {
            integer::encode(size, 5, 0x20, &mut dst);
        }

        for (name, value) in headers {
            self.encode_header(name, value, &mut dst);
        }

        dst
    }

    fn is_sensitive(name: &str) -> bool {
        matches!(
            name,
            "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
        )
    }

    fn encode_header(&mut self, name: &str, value: &str, dst: &mut Vec<u8>) {
        let lowered;
        let name: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            lowered = name.to_ascii_lowercase();
            &lowered
        } else {
            name
        };

        if Self::is_sensitive(name) {
            if let Some((index, _exact)) = table::find_static(name, value) {
                integer::encode(index, 4, 0x10, dst);
                encode_string(value, dst);
            } else {
                dst.push(0x10);
                encode_string(name, dst);
                encode_string(value, dst);
            }
            return;
        }

        let matched = table::find_static(name, value);
        if let Some((index, true)) = matched {
            integer::encode(index, 7, 0x80, dst);
            return;
        }

        let dyn_offset = table::STATIC_TABLE.len();
        let mut dyn_exact = None;
        let mut name_index = matched.map(|(index, _)| index);
        for i in 0..self.dynamic.len() {
            if let Some((n, v)) = self.dynamic.get(i)
                && n.as_ref() == name.as_bytes()
            {
                if v.as_ref() == value.as_bytes() {
                    dyn_exact = Some(dyn_offset + i);
                    break;
                }
                if name_index.is_none() {
                    name_index = Some(dyn_offset + i);
                }
            }
        }

        if let Some(index) = dyn_exact {
            integer::encode(index, 7, 0x80, dst);
            return;
        }
        if let Some(index) = name_index {
            self.encode_literal_indexed_name(index, value, dst);
            return;
        }

        self.encode_literal_new_name(name, value, dst);
    }

    fn encode_literal_indexed_name(&mut self, name_index: usize, value: &str, dst: &mut Vec<u8>) {
        integer::encode(name_index, 6, 0x40, dst);
        encode_string(value, dst);
        let name = self.resolve_name(name_index);
        self.dynamic
            .insert(name, Bytes::copy_from_slice(value.as_bytes()));
    }

    fn encode_literal_new_name(&mut self, name: &str, value: &str, dst: &mut Vec<u8>) {
        dst.push(0x40);
        encode_string(name, dst);
        encode_string(value, dst);
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

fn encode_string(s: &str, dst: &mut Vec<u8>) {
    let huff_len = huffman::encoded_len(s.as_bytes());

    if huff_len < s.len() {
        integer::encode(huff_len, 7, 0x80, dst);
        huffman::encode(s.as_bytes(), dst);
    } else {
        integer::encode(s.len(), 7, 0x00, dst);
        dst.extend_from_slice(s.as_bytes());
    }
}

#[cfg(test)]
mod tests;
