use super::{
    ErrorCode, FRAME_HEADER_LEN, FrameHeader, U31_MAX, WindowUpdateFrame, padded_end_checked,
};

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

#[kani::proof]
fn padded_end_never_underflows_or_exceeds_payload() {
    let buf: [u8; 8] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 8);
    let padded: bool = kani::any();
    let fixed: usize = kani::any_where(|&n| n <= 8);
    let payload = &buf[..len];
    let mandatory = usize::from(padded) + fixed;
    match padded_end_checked(payload, padded, fixed) {
        Ok(end) => {
            assert!(len >= mandatory);
            assert!(end >= mandatory);
            assert!(end <= len);
            let pad_len = if padded { usize::from(payload[0]) } else { 0 };
            assert_eq!(end, len - pad_len);
        }
        Err(ErrorCode::FrameSizeError) => {
            assert!(len < mandatory);
        }
        Err(ErrorCode::ProtocolError) => {
            assert!(padded && len >= mandatory && usize::from(payload[0]) > len - mandatory);
        }
        Err(_) => unreachable!(),
    }
}

#[kani::proof]
fn window_update_encode_masks_reserved_bits() {
    let frame = WindowUpdateFrame {
        stream_id: kani::any(),
        increment: kani::any(),
    };
    let mut wire = Vec::with_capacity(FRAME_HEADER_LEN + 4);
    frame.encode(&mut wire);
    assert_eq!(wire.len(), FRAME_HEADER_LEN + 4);
    assert_eq!(wire[5] & 0x80, 0);
    assert_eq!(wire[FRAME_HEADER_LEN] & 0x80, 0);
    let hdr = FrameHeader::parse(wire[..FRAME_HEADER_LEN].try_into().expect("9 bytes"));
    assert_eq!(hdr.length, 4);
    assert_eq!(hdr.stream_id, frame.stream_id & U31_MAX);
    let inc = u32::from_be_bytes(wire[FRAME_HEADER_LEN..].try_into().expect("4 bytes"));
    assert_eq!(inc, frame.increment & U31_MAX);
}
