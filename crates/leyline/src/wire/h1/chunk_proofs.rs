use super::chunk_size_value;

const LONGEST: usize = 9;

fn expected(line: &[u8]) -> Option<u64> {
    let mut index = 0;
    let mut value = 0u64;
    let mut overflow = false;
    while index < line.len() {
        let Some(digit) = char::from(line[index]).to_digit(16).map(u64::from) else {
            break;
        };
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
