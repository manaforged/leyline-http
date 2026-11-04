use super::code::HUFFMAN_TABLE;
use super::{decode, encode, encoded_len};

const EOS: usize = 256;

/// RFC 7541 section 5.2, read one bit at a time against Appendix B.
fn rfc_7541_decode(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut code, mut len) = (0u32, 0u8);
    for bit in src
        .iter()
        .flat_map(|byte| (0..8).rev().map(move |i| (byte >> i) & 1))
    {
        code = (code << 1) | u32::from(bit);
        len += 1;
        if let Some(symbol) = HUFFMAN_TABLE.iter().position(|&entry| entry == (code, len)) {
            if symbol == EOS {
                return None;
            }
            out.push(symbol as u8);
            (code, len) = (0, 0);
        }
    }
    (len <= 7 && code == (1 << len) - 1).then_some(out)
}

#[test]
fn the_table_is_the_complete_prefix_code_of_rfc_7541_appendix_b() {
    assert_eq!(HUFFMAN_TABLE[EOS], (0x3fff_ffff, 30));
    let kraft: u64 = HUFFMAN_TABLE
        .iter()
        .map(|&(_, len)| 1u64 << (30 - len))
        .sum();
    assert_eq!(
        kraft,
        1 << 30,
        "the code must use every bit pattern exactly once"
    );
    for (a, &(code_a, len_a)) in HUFFMAN_TABLE.iter().enumerate() {
        assert!((5..=30).contains(&len_a) && code_a < 1 << len_a);
        for &(code_b, len_b) in &HUFFMAN_TABLE[a + 1..] {
            let shared = len_a.min(len_b);
            assert_ne!(
                code_a >> (len_a - shared),
                code_b >> (len_b - shared),
                "prefix clash"
            );
        }
    }
}

#[test]
fn every_input_up_to_two_bytes_decodes_as_rfc_7541_section_5_2_says() {
    let mut inputs: Vec<Vec<u8>> = vec![Vec::new()];
    inputs.extend((0..=255u8).map(|a| vec![a]));
    inputs.extend((0..=0xffffu16).map(|ab| ab.to_be_bytes().to_vec()));
    let edges = [
        0x00u8, 0x01, 0x0f, 0x1f, 0x3f, 0x7f, 0x80, 0xc0, 0xf0, 0xf8, 0xfc, 0xfe, 0xff,
    ];
    for a in edges {
        for b in edges {
            for c in edges {
                for d in [0x00u8, 0x7f, 0xfe, 0xff] {
                    inputs.push(vec![a, b, c, d]);
                }
            }
        }
    }
    for input in inputs {
        assert_eq!(
            decode(&input).ok(),
            rfc_7541_decode(&input),
            "input {input:02x?}"
        );
    }
}

#[test]
fn every_string_up_to_two_bytes_round_trips() {
    let mut encoded = Vec::new();
    for value in 0..=0x1_ffffu32 {
        let len = if value > 0xffff { 1 } else { 2 };
        let input = &value.to_be_bytes()[4 - len..];
        encoded.clear();
        encode(input, &mut encoded);
        assert_eq!(encoded.len(), encoded_len(input));
        assert_eq!(decode(&encoded).as_deref(), Ok(input), "input {input:02x?}");
    }
}
