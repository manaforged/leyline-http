use super::{
    HEADER_LEN, MAX_JUMPS, WireError, extract_ech_from_svcb, parse_answer_rr, parse_records,
    skip_name,
};

#[kani::proof]
#[kani::unwind(14)]
fn skip_name_terminates_and_stays_in_bounds() {
    let packet: [u8; 10] = kani::any();
    let len: usize = kani::any_where(|n| *n <= packet.len());
    let start: usize = kani::any_where(|n| *n <= len);
    if let Some(next) = skip_name(&packet[..len], start) {
        assert!(next <= len);
        assert!(next > start);
    }
}

#[kani::proof]
#[kani::unwind(14)]
fn skip_name_enforces_max_jumps_bound() {
    let hops: usize = kani::any_where(|&n| (1..=MAX_JUMPS + 2).contains(&n));
    let mut packet = [0u8; 24];
    packet[0] = 0;
    let mut hop = 0;
    while hop < hops {
        let at = 2 + hop * 2;
        let prev = if hop == 0 { 0 } else { at - 2 };
        packet[at] = 0xC0 | ((prev >> 8) as u8);
        packet[at + 1] = (prev & 0xFF) as u8;
        hop += 1;
    }
    let start = 2 + (hops - 1) * 2;
    let expected = (hops <= MAX_JUMPS).then_some(start + 2);
    assert_eq!(skip_name(&packet, start), expected);
}

#[kani::proof]
#[kani::unwind(12)]
fn extract_ech_from_svcb_never_panics() {
    let rdata: [u8; 16] = kani::any();
    let len: usize = kani::any_where(|n| *n <= rdata.len());
    if let Some((priority, ech)) = extract_ech_from_svcb(&rdata[..len]) {
        assert!(priority > 0);
        assert!(!ech.is_empty());
        assert!(ech.len() < len);
    }
}

#[kani::proof]
#[kani::unwind(15)]
fn parse_answer_rr_never_panics() {
    let packet: [u8; 24] = kani::any();
    let len: usize = kani::any_where(|&n| (HEADER_LEN..=packet.len()).contains(&n));
    let start: usize = kani::any_where(|&n| (HEADER_LEN..=len).contains(&n));
    let slice = &packet[..len];
    if let Ok((_rtype, _rclass, _ttl, rdata, next)) = parse_answer_rr(slice, start) {
        assert!(next <= len);
        assert!(next > start);
        assert!(rdata.len() <= next - start);
    }
}

#[kani::proof]
#[kani::unwind(6)]
fn parse_records_question_never_panics() {
    let mut packet: [u8; 18] = kani::any();
    packet[0] = 0;
    packet[1] = 0;
    packet[2] = 0;
    packet[4] = 0;
    packet[5] = kani::any_where(|&n| n <= 1);
    packet[6] = 0;
    packet[7] = 0;
    packet[8] = 0;
    packet[9] = 0;
    packet[10] = 0;
    packet[11] = 0;
    let len: usize = kani::any_where(|n| *n <= packet.len());
    let slice = &packet[..len];
    let res = parse_records(slice, |_, _, _, _| {
        panic!("ancount == 0 must never emit records");
    });
    if len < 12 {
        assert_eq!(res, Err(WireError::Truncated));
    }
}
