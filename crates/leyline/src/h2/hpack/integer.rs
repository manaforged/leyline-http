//! HPACK integer encoding/decoding (RFC 7541 Section 5.1).

/// Encode an integer with the given prefix size (1-8 bits).
pub fn encode(value: usize, prefix_bits: u8, prefix_pattern: u8, dst: &mut Vec<u8>) {
    let max_prefix = (1 << prefix_bits) - 1;

    if value < max_prefix {
        dst.push(prefix_pattern | value as u8);
    } else {
        dst.push(prefix_pattern | max_prefix as u8);
        let mut remaining = value - max_prefix;
        while remaining >= 128 {
            dst.push((remaining & 0x7F) as u8 | 0x80);
            remaining >>= 7;
        }
        dst.push(remaining as u8);
    }
}

/// Decode an integer from `src` starting at `pos`, with the given prefix size.
pub fn decode(
    first_byte: u8,
    prefix_bits: u8,
    src: &[u8],
    pos: usize,
) -> Result<(usize, usize), &'static str> {
    let max_prefix = (1usize << prefix_bits) - 1;
    let value = (first_byte & max_prefix as u8) as usize;

    if value < max_prefix {
        return Ok((value, 0));
    }

    let mut result = max_prefix;
    let mut shift = 0u32;
    let mut i = pos;

    loop {
        if i >= src.len() {
            return Err("unexpected end of integer");
        }
        let byte = src[i];
        i += 1;

        if shift > 28 {
            return Err("integer overflow");
        }

        result += ((byte & 0x7F) as usize) << shift;
        shift += 7;

        if result > 0x7FFF_FFFF {
            return Err("integer overflow");
        }

        if byte & 0x80 == 0 {
            break;
        }
    }

    Ok((result, i - pos))
}

#[cfg(test)]
mod tests;
