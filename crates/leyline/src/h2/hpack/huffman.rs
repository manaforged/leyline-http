mod code;

use code::HUFFMAN_TABLE;

pub fn encode(src: &[u8], dst: &mut Vec<u8>) {
    let mut bits: u64 = 0;
    let mut bits_left: u8 = 0;

    for &byte in src {
        let (code, len) = HUFFMAN_TABLE[byte as usize];
        bits = (bits << len) | code as u64;
        bits_left += len;

        while bits_left >= 8 {
            bits_left -= 8;
            dst.push((bits >> bits_left) as u8);
        }
    }

    if bits_left > 0 {
        let pad = 8 - bits_left;
        bits = (bits << pad) | ((1u64 << pad) - 1);
        dst.push(bits as u8);
    }
}

pub fn decode(src: &[u8]) -> Result<Vec<u8>, &'static str> {
    let table = decode_table();
    let mut dst = Vec::new();
    let mut acc: u64 = 0;
    let mut acc_bits: u8 = 0;

    for &byte in src {
        acc = (acc << 8) | byte as u64;
        acc_bits += 8;

        while acc_bits >= 5 {
            match decode_symbol(table, acc, acc_bits) {
                Some((symbol, bits_consumed)) => {
                    if symbol == 256 {
                        return Err("EOS symbol in Huffman stream");
                    }
                    dst.push(symbol as u8);
                    acc_bits -= bits_consumed;
                    acc &= (1u64 << acc_bits) - 1;
                }
                None => break,
            }
        }
    }

    if acc_bits > 7 {
        return Err("Huffman padding exceeds 7 bits");
    }
    if acc_bits > 0 {
        let mask = (1u64 << acc_bits) - 1;
        if acc & mask != mask {
            return Err("Huffman padding is not all 1s");
        }
    }

    Ok(dst)
}

#[derive(Clone, Copy)]
struct Fast8 {
    sym: u16,
    len: u8,
}

struct DecodeTable {
    by_length: [Vec<(u32, u16)>; 26],
    fast8: [Fast8; 256],
}

impl DecodeTable {
    fn new() -> Self {
        let mut by_length: [Vec<(u32, u16)>; 26] = Default::default();
        for (sym, &(code, len)) in HUFFMAN_TABLE.iter().enumerate() {
            if (5..=30).contains(&len) {
                by_length[(len - 5) as usize].push((code, sym as u16));
            }
        }
        for bucket in &mut by_length {
            bucket.sort_unstable_by_key(|&(code, _)| code);
        }
        let lookup = |code: u32, len: u8| -> Option<u16> {
            let bucket = &by_length[(len - 5) as usize];
            bucket
                .binary_search_by_key(&code, |&(c, _)| c)
                .ok()
                .map(|idx| bucket[idx].1)
        };
        let mut fast8 = [Fast8 { sym: 0, len: 0 }; 256];
        for (v, slot) in fast8.iter_mut().enumerate() {
            for len in 5u8..=8 {
                let candidate = (v as u32) >> (8 - len);
                if let Some(sym) = lookup(candidate, len) {
                    *slot = Fast8 { sym, len };
                    break;
                }
            }
        }
        Self { by_length, fast8 }
    }

    fn lookup(&self, code: u32, len: u8) -> Option<u16> {
        if !(5..=30).contains(&len) {
            return None;
        }
        let bucket = &self.by_length[(len - 5) as usize];
        bucket
            .binary_search_by_key(&code, |&(c, _)| c)
            .ok()
            .map(|idx| bucket[idx].1)
    }
}

fn decode_table() -> &'static DecodeTable {
    use std::sync::LazyLock;
    static TABLE: LazyLock<DecodeTable> = LazyLock::new(DecodeTable::new);
    &TABLE
}

fn decode_symbol(table: &DecodeTable, acc: u64, acc_bits: u8) -> Option<(u16, u8)> {
    let start = if acc_bits >= 8 {
        let top8 = ((acc >> (acc_bits - 8)) & 0xff) as usize;
        let f = table.fast8[top8];
        if f.len != 0 {
            return Some((f.sym, f.len));
        }
        9
    } else {
        5
    };
    let max_len = acc_bits.min(30);
    for len in start..=max_len {
        let shift = acc_bits - len;
        let candidate = (acc >> shift) as u32;
        if let Some(symbol) = table.lookup(candidate, len) {
            return Some((symbol, len));
        }
    }
    None
}

pub fn encoded_len(src: &[u8]) -> usize {
    let total_bits: usize = src
        .iter()
        .map(|&b| HUFFMAN_TABLE[b as usize].1 as usize)
        .sum();
    total_bits.div_ceil(8)
}

#[cfg(test)]
mod tests;
