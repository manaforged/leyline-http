use super::{FRAME_HEADER_LEN, FrameHeader, U31_MAX};

#[kani::proof]
fn a_frame_header_round_trips_and_ignores_the_reserved_bit() {
    let sent = FrameHeader {
        length: kani::any_where(|len: &u32| *len < 1 << 24),
        frame_type: kani::any(),
        flags: kani::any(),
        stream_id: kani::any(),
    };
    let mut wire = Vec::new();
    sent.encode(&mut wire);
    assert_eq!(wire.len(), FRAME_HEADER_LEN);
    assert_eq!(wire[5] & 0x80, 0);
    let mut flipped: [u8; FRAME_HEADER_LEN] = wire.as_slice().try_into().expect("nine bytes");
    flipped[5] |= 0x80;
    for bytes in [&wire[..], &flipped[..]] {
        let got = FrameHeader::parse(bytes.try_into().expect("nine bytes"));
        assert_eq!(got.length, sent.length);
        assert_eq!(got.frame_type, sent.frame_type);
        assert_eq!(got.flags, sent.flags);
        assert_eq!(got.stream_id, sent.stream_id & U31_MAX);
    }
}
