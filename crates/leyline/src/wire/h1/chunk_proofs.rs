use super::chunk_size_value;

const LONGEST: usize = 9;

fn hex(byte: u8) -> Option<u64> {
    match byte {
        b'0'..=b'9' => Some(u64::from(byte - b'0')),
        b'a'..=b'f' => Some(u64::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u64::from(byte - b'A') + 10),
        _ => None,
    }
}

fn expected(line: &[u8]) -> Option<u64> {
    let mut index = 0;
    let mut value = 0u64;
    let mut overflow = false;
    while index < line.len() {
        let Some(digit) = hex(line[index]) else { break };
        match value.checked_mul(16).and_then(|v| v.checked_add(digit)) {
            Some(next) => value = next,
            None => overflow = true,
        }
        index += 1;
    }
    if index == 0 {
        return None;
    }
    while index < line.len() && (line[index] == b' ' || line[index] == b'\t') {
        index += 1;
    }
    if index < line.len() && line[index] != b';' {
        return None;
    }
    (!overflow).then_some(value)
}

#[kani::proof]
#[kani::unwind(11)]
fn a_chunk_size_follows_rfc_9112_section_7_1() {
    let bytes: [u8; LONGEST] = kani::any();
    let length: usize = kani::any_where(|n| *n <= LONGEST);
    let line = &bytes[..length];
    assert_eq!(chunk_size_value(line), expected(line));
}

#[kani::proof]
#[kani::unwind(20)]
fn a_chunk_size_overflow_is_exact_at_u64_max() {
    const HEX: [u8; 22] = *b"0123456789abcdefABCDEF";
    let mut line = [b'f'; 17];
    let first_idx: usize = kani::any_where(|&i| i < HEX.len());
    let last_idx: usize = kani::any_where(|&i| i < HEX.len());
    let first = HEX[first_idx];
    let last = HEX[last_idx];
    line[0] = first;
    line[16] = last;
    if first == b'0' {
        let low = hex(last).unwrap();
        assert_eq!(chunk_size_value(&line), Some(0xffff_ffff_ffff_fff0 | low));
    } else {
        assert_eq!(chunk_size_value(&line), None);
    }
}
