use super::*;

#[test]
fn encodes_standard_a_and_https_queries() {
    let q = encode_query("example.com.", TYPE_A).unwrap();
    assert_eq!(&q[..12], &[0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&q[12..], b"\x07example\x03com\x00\x00\x01\x00\x01");
    let h = encode_query("secret.test", TYPE_HTTPS).unwrap();
    assert_eq!(&h[h.len() - 4..], &[0, 65, 0, 1]);
    assert_eq!(encode_query("", TYPE_A), Err(WireError::InvalidName));
    assert_eq!(encode_query("a..b", TYPE_A), Err(WireError::InvalidName));
    let too_long = format!("{}.com", "a".repeat(64));
    assert_eq!(encode_query(&too_long, TYPE_A), Err(WireError::InvalidName));
}

#[test]
fn rejects_compression_pointer_loops() {
    let packet = [0xC0u8, 0x00];
    assert_eq!(skip_name(&packet, 0), None);
}

#[test]
fn extracts_lowest_priority_ech_svcb_param() {
    let mut rdata = Vec::new();
    rdata.extend_from_slice(&1u16.to_be_bytes());
    rdata.push(0);
    rdata.extend_from_slice(&1u16.to_be_bytes());
    rdata.extend_from_slice(&3u16.to_be_bytes());
    rdata.extend_from_slice(b"\x02h2");
    rdata.extend_from_slice(&5u16.to_be_bytes());
    rdata.extend_from_slice(&4u16.to_be_bytes());
    rdata.extend_from_slice(b"\xde\xad\xbe\xef");
    assert_eq!(
        extract_ech_from_svcb(&rdata),
        Some((1, &b"\xde\xad\xbe\xef"[..]))
    );
}

#[test]
fn validate_qname_enforces_label_rules_exhaustively() {
    let alphabet: &[&[u8]] = &[b".", b"a", b"-"];
    crate::util::every_input(alphabet, 6, |bytes| {
        let host = std::str::from_utf8(bytes).expect("ascii");
        let trimmed = host.strip_suffix('.').unwrap_or(host);
        let has_empty = trimmed.is_empty() || trimmed.split('.').any(str::is_empty);
        match validate_qname(host) {
            Ok(out) => {
                assert!(!has_empty, "{host:?}");
                assert_eq!(out, trimmed, "{host:?}");
            }
            Err(WireError::InvalidName) => assert!(has_empty, "{host:?}"),
            Err(other) => panic!("unexpected error {other:?} for {host:?}"),
        }
    });
    assert_eq!(validate_qname("café.test"), Err(WireError::InvalidName));
    let max_label = "a".repeat(63);
    assert_eq!(validate_qname(&max_label), Ok(max_label.as_str()));
    let max_name = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(max_name.len(), 253);
    assert_eq!(validate_qname(&max_name), Ok(max_name.as_str()));
    let over_name = format!("{max_name}e");
    assert_eq!(validate_qname(&over_name), Err(WireError::InvalidName));
}

#[test]
fn parse_records_answer_rr_bounds_rdata_exhaustively() {
    for len in 12..=26usize {
        for rdlen in 0..=4u16 {
            let mut packet = vec![0u8; len];
            packet[2] = 0x81;
            packet[3] = 0x80;
            packet[7] = 1;
            if len >= 23 {
                packet[21..23].copy_from_slice(&rdlen.to_be_bytes());
            }
            let mut seen = 0usize;
            let res = parse_records(&packet, |_rtype, _rclass, _ttl, rdata| {
                assert_eq!(rdata.len(), usize::from(rdlen));
                assert!(rdata.len() <= len - 23);
                seen += 1;
            });
            if len >= 23 + usize::from(rdlen) {
                assert_eq!(res, Ok(()), "len={len} rdlen={rdlen}");
                assert_eq!(seen, 1);
            } else if len == 12 {
                assert_eq!(
                    res,
                    Err(WireError::MalformedName),
                    "len={len} rdlen={rdlen}"
                );
                assert_eq!(seen, 0);
            } else {
                assert_eq!(res, Err(WireError::Truncated), "len={len} rdlen={rdlen}");
                assert_eq!(seen, 0);
            }
        }
    }
}

#[test]
fn prop_encode_and_parse_roundtrip_random_a_and_aaaa_packets() {
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next_u64 = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    for case in 0..200u32 {
        let labels = 1 + (next_u64() as usize % 4);
        let mut host = String::new();
        for l in 0..labels {
            if l > 0 {
                host.push('.');
            }
            let len = 1 + (next_u64() as usize % 10);
            for _ in 0..len {
                let ch = b'a' + (next_u64() as u8 % 26);
                host.push(ch as char);
            }
        }
        let query = encode_query(&host, TYPE_A).expect("valid random host");
        let mut pkt = query.clone();
        pkt[2] = 0x81;
        pkt[3] = 0x80;
        let count = 1 + (next_u64() as usize % 4);
        pkt[6..8].copy_from_slice(&(count as u16).to_be_bytes());
        let mut expected_ips = Vec::with_capacity(count);
        let mut expected_min_ttl = u32::MAX;
        for _ in 0..count {
            let ttl = 1 + (next_u64() as u32 % 3600);
            expected_min_ttl = expected_min_ttl.min(ttl);
            pkt.extend_from_slice(&[0xc0, 0x0c]);
            if next_u64() & 1 == 0 {
                let octets = [192, 0, 2, 1 + (next_u64() as u8 % 250)];
                expected_ips.push(IpAddr::V4(Ipv4Addr::from(octets)));
                pkt.extend_from_slice(&TYPE_A.to_be_bytes());
                pkt.extend_from_slice(&1u16.to_be_bytes());
                pkt.extend_from_slice(&ttl.to_be_bytes());
                pkt.extend_from_slice(&4u16.to_be_bytes());
                pkt.extend_from_slice(&octets);
            } else {
                let mut octets = [0u8; 16];
                octets[0] = 0x20;
                octets[1] = 0x01;
                octets[2] = 0x0d;
                octets[3] = 0xb8;
                octets[15] = 1 + (next_u64() as u8 % 250);
                expected_ips.push(IpAddr::V6(Ipv6Addr::from(octets)));
                pkt.extend_from_slice(&TYPE_AAAA.to_be_bytes());
                pkt.extend_from_slice(&1u16.to_be_bytes());
                pkt.extend_from_slice(&ttl.to_be_bytes());
                pkt.extend_from_slice(&16u16.to_be_bytes());
                pkt.extend_from_slice(&octets);
            }
        }
        let parsed = parse_response(&pkt).unwrap_or_else(|e| panic!("case {case}: {e:?}"));
        assert_eq!(parsed.addrs, expected_ips, "case {case} IPs");
        assert_eq!(parsed.min_ttl, Some(expected_min_ttl), "case {case} TTL");
    }
}
