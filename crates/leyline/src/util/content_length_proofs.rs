use super::content_length_value;

const LONGEST: usize = 7;

fn expected(bytes: &[u8]) -> Option<u64> {
    let blank = |byte: u8| byte == b' ' || byte == b'\t';
    let (mut start, mut end) = (0, bytes.len());
    while start < end && blank(bytes[start]) {
        start += 1;
    }
    while end > start && blank(bytes[end - 1]) {
        end -= 1;
    }
    if start == end {
        return None;
    }
    let mut value = 0u128;
    let mut index = start;
    while index < end {
        if !bytes[index].is_ascii_digit() {
            return None;
        }
        value = value * 10 + u128::from(bytes[index] - b'0');
        index += 1;
    }
    (value <= u128::from(u64::MAX)).then_some(value as u64)
}

#[kani::proof]
#[kani::unwind(9)]
fn a_content_length_follows_rfc_9110_section_8_6() {
    let bytes: [u8; LONGEST] = kani::any();
    let length: usize = kani::any_where(|n| *n <= LONGEST);
    let value = &bytes[..length];
    assert_eq!(content_length_value(value), expected(value));
}

#[kani::proof]
#[kani::unwind(23)]
fn a_content_length_overflow_is_exact_at_u64_max() {
    let high = *b"18446744073709551";
    let low: [u8; 3] = kani::any();
    kani::assume(low.iter().all(u8::is_ascii_digit));
    let mut value = [0; 20];
    value[..17].copy_from_slice(&high);
    value[17..].copy_from_slice(&low);
    let below =
        u64::from(low[0] - b'0') * 100 + u64::from(low[1] - b'0') * 10 + u64::from(low[2] - b'0');
    let expected = (below <= 615).then(|| 18_446_744_073_709_551_000 + below);
    assert_eq!(content_length_value(&value), expected);
}
