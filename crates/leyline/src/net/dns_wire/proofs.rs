use super::{MAX_JUMPS, WireError, extract_ech_from_svcb, parse_records, skip_name};

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
    let mut packet = [0u8; 20];
    packet[0] = 0;
    for hop in 0..=MAX_JUMPS {
        let at = 2 + hop * 2;
        let prev = if hop == 0 { 0 } else { at - 2 };
        packet[at] = 0xC0 | ((prev >> 8) as u8);
        packet[at + 1] = (prev & 0xFF) as u8;
    }
    let at_limit = 2 + (MAX_JUMPS - 1) * 2;
    assert_eq!(skip_name(&packet, at_limit), Some(at_limit + 2));
    let over_limit = 2 + MAX_JUMPS * 2;
    assert_eq!(skip_name(&packet, over_limit), None);
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
#[kani::unwind(3)]
fn parse_records_answer_rr_never_panics() {
    let mut packet: [u8; 24] = kani::any();
    packet[4] = 0;
    packet[5] = 0;
    packet[6] = 0;
    packet[7] = 1;
    packet[12] = 0;
    let len: usize = kani::any_where(|n| (22..=24).contains(n));
    let slice = &packet[..len];
    let mut seen = 0usize;
    let res = parse_records(slice, |_rtype, _rclass, _ttl, rdata| {
        assert!(rdata.len() <= len - 23);
        seen += 1;
    });
    if len < 23 {
        assert!(res.is_err());
        assert_eq!(seen, 0);
    }
}

#[kani::proof]
#[kani::unwind(4)]
fn parse_records_question_never_panics() {
    let mut packet: [u8; 15] = kani::any();
    packet[4] = 0;
    packet[5] = kani::any_where(|&n| n <= 1);
    packet[6] = 0;
    packet[7] = 0;
    let len: usize = kani::any_where(|n| *n <= packet.len());
    let slice = &packet[..len];
    let res = parse_records(slice, |_, _, _, _| {
        panic!("ancount == 0 must never emit records");
    });
    if len < 12 {
        assert_eq!(res, Err(WireError::Truncated));
    }
}
