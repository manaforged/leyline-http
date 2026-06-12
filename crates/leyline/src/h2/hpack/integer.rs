//! HPACK integer encoding/decoding (RFC 7541 Section 5.1).
//!
//! Integers start at arbitrary bit positions within a byte. The first
//! byte shares its bits with a prefix (flags/type indicator). If the
//! integer value fits in the remaining bits, it's a single byte. Otherwise
//! it spills into continuation bytes (7 bits each, MSB = continue).

/// Encode an integer with the given prefix size (1-8 bits).
///
/// `prefix_bits` is how many bits of the first byte are available for the integer.
/// `prefix_pattern` is the value of the bits ABOVE the integer in the first byte.
///
/// Example: For an indexed header (prefix = 7 bits, top bit = 1):
///   encode(62, 7, 0x80) → [0x80 | 62] = [0xBE]
///   encode(200, 7, 0x80) → [0xFF, 200-127] = [0xFF, 0x49]
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
///
/// Returns `(value, bytes_consumed)` or an error if the encoding is malformed.
///
/// The first byte at `src[pos]` has already had its prefix bits masked off
/// by the caller — `first_byte` is just the integer portion.
pub fn decode(
    first_byte: u8,
    prefix_bits: u8,
    src: &[u8],
    pos: usize,
) -> Result<(usize, usize), &'static str> {
    let max_prefix = (1usize << prefix_bits) - 1;
    let value = (first_byte & max_prefix as u8) as usize;

    if value < max_prefix {
        return Ok((value, 0)); // fits in first byte, no additional bytes consumed
    }

    // Multi-byte integer.
    let mut result = max_prefix;
    let mut shift = 0u32;
    let mut i = pos;

    loop {
        if i >= src.len() {
            return Err("unexpected end of integer");
        }
        let byte = src[i];
        i += 1;

        // Continuation-length check: shift > 28 means a sixth byte,
        // which no value we accept can need.
        if shift > 28 {
            return Err("integer overflow");
        }

        result += ((byte & 0x7F) as usize) << shift;
        shift += 7;

        // Value check: five bytes within the shift guard can still
        // assemble past 2^31-1 on 64-bit. HPACK integers index tables
        // and size strings; the HTTP/2 ceiling is 2^31-1 and anything
        // larger is an attacker-chosen allocation size.
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
mod tests {
    use super::*;

    #[test]
    fn encode_small_value() {
        // RFC 7541 C.1.1: encode 10 with 5-bit prefix
        let mut buf = Vec::new();
        encode(10, 5, 0x00, &mut buf);
        assert_eq!(buf, vec![0x0A]);
    }

    #[test]
    fn encode_prefix_boundary() {
        // RFC 7541 C.1.2: encode 1337 with 5-bit prefix
        let mut buf = Vec::new();
        encode(1337, 5, 0x00, &mut buf);
        assert_eq!(buf, vec![0x1F, 0x9A, 0x0A]);
    }

    #[test]
    fn encode_at_max_prefix() {
        // RFC 7541 C.1.3: encode 42 starting on byte boundary (8-bit prefix)
        let mut buf = Vec::new();
        encode(42, 8, 0x00, &mut buf);
        assert_eq!(buf, vec![42]);
    }

    #[test]
    fn decode_small_value() {
        let (val, consumed) = decode(10, 5, &[], 0).unwrap();
        assert_eq!(val, 10);
        assert_eq!(consumed, 0);
    }

    #[test]
    fn decode_multi_byte() {
        // 1337 with 5-bit prefix: first byte = 0x1F (31), then 0x9A, 0x0A
        let src = [0x9A, 0x0A];
        let (val, consumed) = decode(0x1F, 5, &src, 0).unwrap();
        assert_eq!(val, 1337);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn roundtrip_various_values() {
        for &(value, prefix_bits) in &[
            (0, 5),
            (1, 5),
            (30, 5),
            (31, 5),
            (127, 7),
            (128, 7),
            (255, 8),
            (256, 8),
            (1337, 5),
            (65535, 4),
            (0, 1),
        ] {
            let mut buf = Vec::new();
            encode(value, prefix_bits, 0x00, &mut buf);

            let first = buf[0];
            let (decoded, _) = decode(first, prefix_bits, &buf[1..], 0).unwrap();
            assert_eq!(
                decoded, value,
                "roundtrip failed for value={value}, prefix={prefix_bits}"
            );
        }
    }

    #[test]
    fn decode_overflow_rejected() {
        // A very long continuation sequence that would overflow.
        let src = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        let result = decode(0x1F, 5, &src, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_rejects_values_past_u31_max() {
        // Five continuation bytes all clear the shift<=28 guard yet can
        // assemble a value past 2^31-1 on a 64-bit usize. HPACK integers
        // index tables and size strings; nothing legitimate exceeds the
        // HTTP/2 2^31-1 ceiling, and letting 2^32+ through hands
        // downstream code an attacker-chosen allocation size.
        let mut src = Vec::new();
        let mut rem: u64 = (1u64 << 32) - 31; // encodes 2^32 with a 5-bit prefix
        while rem >= 128 {
            src.push((rem & 0x7F) as u8 | 0x80);
            rem >>= 7;
        }
        src.push(rem as u8);
        assert!(decode(0x1F, 5, &src, 0).is_err(), "2^32 must be rejected");
    }

    #[test]
    fn decode_accepts_max_legal_value() {
        let mut buf = Vec::new();
        encode(0x7FFF_FFFF, 5, 0x00, &mut buf);
        let (val, _) = decode(buf[0], 5, &buf[1..], 0).unwrap();
        assert_eq!(val, 0x7FFF_FFFF);
    }
}
