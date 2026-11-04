use super::{decode, encode};

const MAX_HPACK_INTEGER: usize = 0x7FFF_FFFF;

fn prefix_bits() -> u8 {
    kani::any_where(|bits: &u8| (1..=8).contains(bits))
}

#[kani::proof]
#[kani::unwind(7)]
fn every_hpack_integer_round_trips() {
    let value: usize = kani::any_where(|value: &usize| *value <= MAX_HPACK_INTEGER);
    let bits = prefix_bits();
    let mask = ((1u16 << bits) - 1) as u8;
    let pattern = kani::any::<u8>() & !mask;
    let mut encoded = Vec::new();
    encode(value, bits, pattern, &mut encoded);
    assert!(encoded.len() <= 6);
    assert_eq!(encoded[0] & !mask, pattern);
    assert_eq!(
        decode(encoded[0], bits, &encoded, 1),
        Ok((value, encoded.len() - 1))
    );
}

#[kani::proof]
#[kani::unwind(8)]
fn arbitrary_bytes_never_decode_past_the_hpack_limit() {
    let bytes: [u8; 6] = kani::any();
    let len: usize = kani::any_where(|len: &usize| *len <= bytes.len());
    if let Ok((value, used)) = decode(kani::any(), prefix_bits(), &bytes[..len], 0) {
        assert!(value <= MAX_HPACK_INTEGER);
        assert!(used <= len);
    }
}
