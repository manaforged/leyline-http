const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) fn digest(parts: &[&[u8]]) -> u64 {
    let mut hash = OFFSET;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    };
    for part in parts {
        let kept: Vec<u8> = part.iter().copied().filter(|&b| b != b'\r').collect();
        for byte in (kept.len() as u64).to_le_bytes() {
            feed(byte);
        }
        for byte in kept {
            feed(byte);
        }
    }
    hash
}
