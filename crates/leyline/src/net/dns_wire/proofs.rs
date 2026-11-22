use super::{TYPE_A, encode_query, extract_ech_from_svcb, parse_response, skip_name};

#[kani::proof]
#[kani::unwind(20)]
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
#[kani::unwind(8)]
fn parse_response_rejects_short_packets_and_never_panics() {
    let packet: [u8; 20] = kani::any();
    let len: usize = kani::any_where(|n| *n <= packet.len());
    let res = parse_response(&packet[..len]);
    if len < 12 {
        assert!(res.is_err());
    }
}

#[kani::proof]
#[kani::unwind(10)]
fn encode_query_validates_label_bounds() {
    assert!(encode_query("", TYPE_A).is_err());
    assert!(encode_query("a..b", TYPE_A).is_err());
    let ok = encode_query("a.b", TYPE_A).expect("valid short domain");
    assert_eq!(ok.len(), 12 + 5 + 4);
    assert_eq!(ok[2], 0x01);
    assert_eq!(ok[5], 0x01);
}
