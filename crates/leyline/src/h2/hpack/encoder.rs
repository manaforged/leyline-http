use std::hash::BuildHasher;

use bytes::Bytes;
use foldhash::HashMap;

use super::huffman;
use super::integer;
use super::table::{self, DynamicTable};

#[derive(Default)]
pub struct Encoder {
    dynamic: DynamicTable,
    pending_size_update: Option<usize>,
    entries: HashMap<u64, u64>,
    names: HashMap<u64, u64>,
    block: Vec<u8>,
    slots: Vec<Option<Hit>>,
}

#[derive(Clone, Copy)]
enum Hit {
    Static(usize),
    Dynamic(u64),
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_max_table_size(&mut self, size: usize) {
        self.dynamic.set_max_size(size);
        self.pending_size_update = Some(size);
    }

    #[cfg(any(test, feature = "bench-internals"))]
    pub fn encode_header_block(&mut self, headers: &[(&str, &str)]) -> Vec<u8> {
        self.encode_block(headers.iter().copied()).to_vec()
    }

    pub fn encode_block<'a>(&mut self, headers: impl Iterator<Item = (&'a str, &'a str)>) -> &[u8] {
        let mut dst = std::mem::take(&mut self.block);
        dst.clear();

        if let Some(size) = self.pending_size_update.take() {
            integer::encode(size, 5, 0x20, &mut dst);
        }

        for (slot, (name, value)) in headers.enumerate() {
            let remembered = self.slots.get(slot).copied().flatten();
            if let Some(index) = remembered.and_then(|hit| self.indexed(hit, name, value)) {
                integer::encode(index, 7, 0x80, &mut dst);
                continue;
            }
            let hit = self.encode_header(name, value, &mut dst);
            if slot >= self.slots.len() {
                self.slots.resize(slot + 1, None);
            }
            self.slots[slot] = hit;
        }

        self.block = dst;
        &self.block
    }

    fn indexed(&self, hit: Hit, name: &str, value: &str) -> Option<usize> {
        match hit {
            Hit::Static(index) => {
                let &(n, v) = table::STATIC_TABLE.get(index)?;
                (n == name && v == value).then_some(index)
            }
            Hit::Dynamic(at) => {
                let i = self.dynamic.position(at)?;
                let (n, v) = self.dynamic.get(i)?;
                (n.as_ref() == name.as_bytes() && v.as_ref() == value.as_bytes())
                    .then_some(table::STATIC_TABLE.len() + i)
            }
        }
    }

    fn encode_header(&mut self, name: &str, value: &str, dst: &mut Vec<u8>) -> Option<Hit> {
        let lowered;
        let name: &str = if crate::util::has_ascii_uppercase(name) {
            lowered = name.to_ascii_lowercase();
            &lowered
        } else {
            name
        };

        if crate::util::sensitive_header(name) {
            if let Some((index, _exact)) = table::find_static(name, value) {
                integer::encode(index, 4, 0x10, dst);
                encode_string(value, dst);
            } else {
                dst.push(0x10);
                encode_string(name, dst);
                encode_string(value, dst);
            }
            return None;
        }

        let matched = table::find_static(name, value);
        if let Some((index, true)) = matched {
            integer::encode(index, 7, 0x80, dst);
            return Some(Hit::Static(index));
        }

        let dyn_offset = table::STATIC_TABLE.len();
        let entry_key = self.entries.hasher().hash_one((name, value));
        let found = self.entries.get(&entry_key).copied().and_then(|at| {
            let i = self.dynamic.position(at)?;
            let (n, v) = self.dynamic.get(i)?;
            (n.as_ref() == name.as_bytes() && v.as_ref() == value.as_bytes()).then_some((i, at))
        });
        if let Some((i, at)) = found {
            integer::encode(dyn_offset + i, 7, 0x80, dst);
            return Some(Hit::Dynamic(at));
        }
        let name_key = self.names.hasher().hash_one(name);
        let name_index = matched.map(|(index, _)| index).or_else(|| {
            let at = *self.names.get(&name_key)?;
            let i = self.dynamic.position(at)?;
            let (n, _) = self.dynamic.get(i)?;
            (n.as_ref() == name.as_bytes()).then_some(dyn_offset + i)
        });
        let at = self.dynamic.inserted();
        self.entries.insert(entry_key, at);
        self.names.insert(name_key, at);
        self.forget_evicted();
        if let Some(index) = name_index {
            self.encode_literal_indexed_name(index, value, dst);
        } else {
            self.encode_literal_new_name(name, value, dst);
        }
        Some(Hit::Dynamic(at))
    }

    fn forget_evicted(&mut self) {
        let live = self.dynamic.len() + 1;
        if self.entries.len() <= 4 * live + 64 {
            return;
        }
        let first = self.dynamic.inserted().saturating_sub(live as u64);
        self.entries.retain(|_, at| *at >= first);
        self.names.retain(|_, at| *at >= first);
    }

    fn encode_literal_indexed_name(&mut self, name_index: usize, value: &str, dst: &mut Vec<u8>) {
        integer::encode(name_index, 6, 0x40, dst);
        encode_string(value, dst);
        let name = table::lookup(name_index, &self.dynamic)
            .map(|(name, _)| name)
            .unwrap_or_default();
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
