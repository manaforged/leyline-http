// Copyright (C) 2026, Manaforge Technologies, LLC.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//     * Redistributions of source code must retain the above copyright notice,
//       this list of conditions and the following disclaimer.
//
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
// IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO,
// THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
// PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
// CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
// EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
// PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
// LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
// NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::VecDeque;
use std::sync::Arc;

use super::Error;
use super::Result;
use super::decoder::decode_int;
use super::encode_int;

const ENTRY_OVERHEAD: u64 = 32;
const DECODER_STREAM_BACKLOG: usize = 64 * 1024;

type Field = Arc<[u8]>;

#[derive(Default)]
pub(super) struct DynamicTable {
    entries: VecDeque<(Field, Field)>,
    size: u64,
    capacity: u64,
    max_capacity: u64,
    inserted: u64,
}

impl DynamicTable {
    pub(super) fn new(max_capacity: u64) -> Self {
        DynamicTable {
            max_capacity,
            ..Default::default()
        }
    }

    pub(super) fn insert_count(&self) -> u64 {
        self.inserted
    }

    pub(super) fn max_capacity(&self) -> u64 {
        self.max_capacity
    }

    pub(super) fn get(&self, absolute: u64) -> Result<(&[u8], &[u8])> {
        self.entry(absolute)
            .map(|(name, value)| (name.as_ref(), value.as_ref()))
    }

    fn entry(&self, absolute: u64) -> Result<&(Field, Field)> {
        let dropped = self.inserted - self.entries.len() as u64;
        if absolute < dropped || absolute >= self.inserted {
            return Err(Error::InvalidDynamicTableIndex);
        }
        Ok(&self.entries[(absolute - dropped) as usize])
    }

    fn relative(&self, index: u64) -> Result<u64> {
        index
            .checked_add(1)
            .and_then(|n| self.inserted.checked_sub(n))
            .ok_or(Error::InvalidDynamicTableIndex)
    }

    fn set_capacity(&mut self, capacity: u64) -> Result<()> {
        if capacity > self.max_capacity {
            return Err(Error::EncoderStream);
        }
        self.capacity = capacity;
        self.evict(0);
        Ok(())
    }

    fn insert(&mut self, name: Field, value: Field) -> Result<()> {
        let size = entry_size(&name, &value);
        if size > self.capacity {
            return Err(Error::EncoderStream);
        }
        self.evict(size);
        self.size += size;
        self.entries.push_back((name, value));
        self.inserted += 1;
        Ok(())
    }

    fn evict(&mut self, room: u64) {
        while self.size + room > self.capacity {
            let Some((name, value)) = self.entries.pop_front() else {
                break;
            };
            self.size -= entry_size(&name, &value);
        }
    }

    pub(super) fn required_insert_count(&self, encoded: u64) -> Result<u64> {
        if encoded == 0 {
            return Ok(0);
        }
        let max_entries = self.max_capacity / ENTRY_OVERHEAD;
        let full_range = 2 * max_entries;
        if encoded > full_range {
            return Err(Error::InvalidRequiredInsertCount);
        }
        let max_value = self.inserted + max_entries;
        let max_wrapped = (max_value / full_range) * full_range;
        let mut count = max_wrapped + encoded - 1;
        if count > max_value {
            if count <= full_range {
                return Err(Error::InvalidRequiredInsertCount);
            }
            count -= full_range;
        }
        if count == 0 {
            return Err(Error::InvalidRequiredInsertCount);
        }
        Ok(count)
    }
}

fn entry_size(name: &[u8], value: &[u8]) -> u64 {
    (name.len() + value.len()) as u64 + ENTRY_OVERHEAD
}

enum Instruction {
    SetCapacity(u64),
    InsertStaticName(u64, Vec<u8>),
    InsertDynamicName(u64, Vec<u8>),
    InsertLiteral(Vec<u8>, Vec<u8>),
    Duplicate(u64),
}

#[derive(Default)]
pub(super) struct EncoderStream {
    pending: Vec<u8>,
}

impl EncoderStream {
    pub(super) fn process(&mut self, table: &mut DynamicTable, buf: &[u8]) -> Result<u64> {
        self.pending.extend_from_slice(buf);
        let limit = table.max_capacity() as usize + 4096;
        let before = table.insert_count();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            let mut b = octets::Octets::with_slice(&self.pending[consumed..]);
            let instruction = match parse_instruction(&mut b, limit) {
                Ok(v) => v,
                Err(Error::BufferTooShort) => break,
                Err(Error::IntegerOverflow) => return Err(Error::EncoderStream),
                Err(e) => return Err(e),
            };
            consumed += b.off();
            apply(table, instruction)?;
        }
        self.pending.drain(..consumed);
        if self.pending.len() > limit {
            return Err(Error::EncoderStream);
        }
        Ok(table.insert_count() - before)
    }
}

fn parse_instruction(b: &mut octets::Octets, limit: usize) -> Result<Instruction> {
    let first = b.peek_u8()?;
    if first & 0x80 == 0x80 {
        let index = decode_int(b, 6)?;
        let value = decode_prefixed_str(b, 7, limit)?;
        return Ok(if first & 0x40 == 0x40 {
            Instruction::InsertStaticName(index, value)
        } else {
            Instruction::InsertDynamicName(index, value)
        });
    }
    if first & 0x40 == 0x40 {
        let name = decode_prefixed_str(b, 5, limit)?;
        let value = decode_prefixed_str(b, 7, limit)?;
        return Ok(Instruction::InsertLiteral(name, value));
    }
    if first & 0x20 == 0x20 {
        return Ok(Instruction::SetCapacity(decode_int(b, 5)?));
    }
    Ok(Instruction::Duplicate(decode_int(b, 5)?))
}

fn apply(table: &mut DynamicTable, instruction: Instruction) -> Result<()> {
    match instruction {
        Instruction::SetCapacity(capacity) => table.set_capacity(capacity),
        Instruction::InsertStaticName(index, value) => {
            let (name, _) = super::decoder::lookup_static(index)?;
            table.insert(name.into(), value.into())
        }
        Instruction::InsertDynamicName(index, value) => {
            let absolute = table.relative(index)?;
            let name = Arc::clone(&table.entry(absolute)?.0);
            table.insert(name, value.into())
        }
        Instruction::InsertLiteral(name, value) => table.insert(name.into(), value.into()),
        Instruction::Duplicate(index) => {
            let absolute = table.relative(index)?;
            let (name, value) = table.entry(absolute)?.clone();
            table.insert(name, value)
        }
    }
    .map_err(|e| match e {
        Error::InvalidStaticTableIndex | Error::InvalidDynamicTableIndex => Error::EncoderStream,
        e => e,
    })
}

fn decode_prefixed_str(b: &mut octets::Octets, prefix: usize, max_len: usize) -> Result<Vec<u8>> {
    let huffman = b.peek_u8()? & (1 << prefix) != 0;
    let len = decode_int(b, prefix)? as usize;
    if len > max_len {
        return Err(Error::HeaderListTooLarge);
    }
    let mut raw = b.get_bytes(len)?;
    if huffman {
        return raw
            .get_huffman_decoded_with_max_length(max_len)
            .map_err(|_| Error::InvalidHuffmanEncoding);
    }
    Ok(raw.to_vec())
}

#[derive(Default)]
pub(super) struct DecoderStream {
    out: Vec<u8>,
}

impl DecoderStream {
    fn put(&mut self, v: u64, first: u8, prefix: usize) {
        let mut buf = [0; 16];
        let mut b = octets::OctetsMut::with_slice(&mut buf);
        if encode_int(v, first, prefix, &mut b).is_ok() {
            let off = b.off();
            self.out.extend_from_slice(&buf[..off]);
        }
    }

    pub(super) fn section_ack(&mut self, stream_id: u64) {
        self.put(stream_id, 0x80, 7);
    }

    pub(super) fn stream_cancel(&mut self, stream_id: u64) {
        self.put(stream_id, 0x40, 6);
    }

    pub(super) fn insert_count_increment(&mut self, increment: u64) {
        if increment > 0 {
            self.put(increment, 0x00, 6);
        }
    }

    pub(super) fn is_backlogged(&self) -> bool {
        self.out.len() > DECODER_STREAM_BACKLOG
    }

    pub(super) fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    pub(super) fn requeue(&mut self, bytes: &[u8]) {
        let mut out = bytes.to_vec();
        out.append(&mut self.out);
        self.out = out;
    }
}
