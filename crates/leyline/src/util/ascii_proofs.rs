use super::has_ascii_uppercase_bytes;

#[kani::proof]
#[kani::unwind(18)]
fn has_ascii_uppercase_swar_matches_byte_scan() {
    let buf: [u8; 16] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 16);
    let slice = &buf[..len];
    assert_eq!(
        has_ascii_uppercase_bytes(slice),
        slice.iter().any(u8::is_ascii_uppercase)
    );
}
