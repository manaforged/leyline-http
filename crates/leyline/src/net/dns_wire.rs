use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub(crate) const TYPE_A: u16 = 1;
pub(crate) const TYPE_AAAA: u16 = 28;
pub(crate) const TYPE_SVCB: u16 = 64;
pub(crate) const TYPE_HTTPS: u16 = 65;

const CLASS_IN: u16 = 1;
const SVC_PARAM_ECH: u16 = 5;
const HEADER_LEN: usize = 12;
const MAX_NAME_LEN: usize = 255;
const MAX_LABEL_LEN: usize = 63;
const MAX_JUMPS: usize = 8;
const MAX_LABELS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WireError {
    InvalidName,
    Truncated,
    NotResponse,
    NxDomain,
    Rcode(u8),
    MalformedName,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DnsAnswer {
    pub(crate) addrs: Vec<IpAddr>,
    pub(crate) ech_config_list: Option<Vec<u8>>,
    pub(crate) min_ttl: Option<u32>,
}

pub(crate) fn validate_qname(host: &str) -> Result<&str, WireError> {
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() {
        return Err(WireError::InvalidName);
    }
    let mut wire_len = 1usize;
    let mut label_len = 0usize;
    for &b in bytes {
        if !b.is_ascii() {
            return Err(WireError::InvalidName);
        }
        if b == b'.' {
            if label_len == 0 || label_len > MAX_LABEL_LEN {
                return Err(WireError::InvalidName);
            }
            wire_len = wire_len
                .checked_add(1 + label_len)
                .ok_or(WireError::InvalidName)?;
            label_len = 0;
        } else {
            label_len += 1;
        }
    }
    if label_len == 0 || label_len > MAX_LABEL_LEN {
        return Err(WireError::InvalidName);
    }
    wire_len = wire_len
        .checked_add(1 + label_len)
        .ok_or(WireError::InvalidName)?;
    if wire_len > MAX_NAME_LEN {
        return Err(WireError::InvalidName);
    }
    Ok(trimmed)
}

pub(crate) fn encode_query(host: &str, qtype: u16) -> Result<Vec<u8>, WireError> {
    let trimmed = validate_qname(host)?;
    let mut packet = Vec::with_capacity(HEADER_LEN + trimmed.len() + 6);
    packet.extend_from_slice(&[
        0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    for label in trimmed.split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(packet)
}

#[cfg(any(test, kani))]
pub(crate) fn skip_name(packet: &[u8], start: usize) -> Option<usize> {
    skip_name_with_floor(packet, start, 0)
}

fn skip_name_with_floor(packet: &[u8], start: usize, min_target: usize) -> Option<usize> {
    let mut pos = start;
    let mut after_first_jump: Option<usize> = None;
    let mut jump_limit = start;
    let mut jumps = 0usize;
    let mut labels = 0usize;
    while labels < MAX_LABELS {
        let byte = *packet.get(pos)?;
        match byte & 0xC0 {
            0x00 => {
                if byte == 0 {
                    return Some(after_first_jump.unwrap_or(pos + 1));
                }
                let next = pos.checked_add(1)?.checked_add(usize::from(byte))?;
                if next > packet.len() {
                    return None;
                }
                pos = next;
                labels += 1;
            }
            0xC0 => {
                let lo = *packet.get(pos.checked_add(1)?)?;
                let target = (usize::from(byte & 0x3F) << 8) | usize::from(lo);
                let ceiling = if after_first_jump.is_none() {
                    pos
                } else {
                    jump_limit
                };
                if target < min_target
                    || target >= ceiling
                    || target >= packet.len()
                    || jumps >= MAX_JUMPS
                {
                    return None;
                }
                if after_first_jump.is_none() {
                    after_first_jump = Some(pos + 2);
                }
                jump_limit = target;
                pos = target;
                jumps += 1;
                labels += 1;
            }
            _ => return None,
        }
    }
    None
}

fn skip_uncompressed_name(buf: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;
    let mut labels = 0usize;
    while labels < MAX_LABELS {
        let len = *buf.get(pos)?;
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xC0 != 0 {
            return None;
        }
        pos = pos.checked_add(1)?.checked_add(usize::from(len))?;
        if pos > buf.len() {
            return None;
        }
        labels += 1;
    }
    None
}

pub(crate) fn extract_ech_from_svcb(rdata: &[u8]) -> Option<(u16, &[u8])> {
    if rdata.len() < 3 {
        return None;
    }
    let priority = u16::from_be_bytes([rdata[0], rdata[1]]);
    if priority == 0 {
        return None;
    }
    let mut pos = skip_uncompressed_name(rdata, 2)?;
    while pos < rdata.len() {
        let header_end = pos.checked_add(4)?;
        if header_end > rdata.len() {
            return None;
        }
        let key = u16::from_be_bytes([rdata[pos], rdata[pos + 1]]);
        let val_len = usize::from(u16::from_be_bytes([rdata[pos + 2], rdata[pos + 3]]));
        let val_end = header_end.checked_add(val_len)?;
        if val_end > rdata.len() {
            return None;
        }
        if key == SVC_PARAM_ECH && val_len > 0 {
            return Some((priority, &rdata[header_end..val_end]));
        }
        pos = val_end;
    }
    None
}

fn read_u16(packet: &[u8], pos: usize) -> Option<u16> {
    let bytes = packet.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(packet: &[u8], pos: usize) -> Option<u32> {
    let bytes = packet.get(pos..pos.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(crate) fn parse_header(packet: &[u8]) -> Result<(u16, u16), WireError> {
    if packet.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    if flags & 0x8000 == 0 {
        return Err(WireError::NotResponse);
    }
    let rcode = (flags & 0x000F) as u8;
    if rcode == 3 {
        return Err(WireError::NxDomain);
    }
    if rcode != 0 {
        return Err(WireError::Rcode(rcode));
    }
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    let ancount = u16::from_be_bytes([packet[6], packet[7]]);
    Ok((qdcount, ancount))
}

pub(crate) fn parse_answer_rr(
    packet: &[u8],
    pos: usize,
) -> Result<(u16, u16, u32, &[u8], usize), WireError> {
    let pos = skip_name_with_floor(packet, pos, HEADER_LEN).ok_or(WireError::MalformedName)?;
    let rtype = read_u16(packet, pos).ok_or(WireError::Truncated)?;
    let rclass = read_u16(packet, pos + 2).ok_or(WireError::Truncated)?;
    let ttl = read_u32(packet, pos + 4).ok_or(WireError::Truncated)?;
    let rdlength = usize::from(read_u16(packet, pos + 8).ok_or(WireError::Truncated)?);
    let rdata_start = pos.checked_add(10).ok_or(WireError::Truncated)?;
    let rdata_end = rdata_start
        .checked_add(rdlength)
        .ok_or(WireError::Truncated)?;
    let rdata = packet
        .get(rdata_start..rdata_end)
        .ok_or(WireError::Truncated)?;
    Ok((rtype, rclass, ttl, rdata, rdata_end))
}

pub(crate) fn parse_records<'a>(
    packet: &'a [u8],
    mut on_record: impl FnMut(u16, u16, u32, &'a [u8]),
) -> Result<(), WireError> {
    let (qdcount, ancount) = parse_header(packet)?;
    let mut pos = HEADER_LEN;
    for _ in 0..qdcount {
        pos = skip_name_with_floor(packet, pos, HEADER_LEN).ok_or(WireError::MalformedName)?;
        pos = pos.checked_add(4).ok_or(WireError::Truncated)?;
        if pos > packet.len() {
            return Err(WireError::Truncated);
        }
    }
    for _ in 0..ancount {
        let (rtype, rclass, ttl, rdata, next) = parse_answer_rr(packet, pos)?;
        pos = next;
        on_record(rtype, rclass, ttl, rdata);
    }
    Ok(())
}

pub(crate) fn update_min_ttl(slot: &mut Option<u32>, ttl: u32) {
    *slot = Some(slot.map_or(ttl, |cur| cur.min(ttl)));
}

pub(crate) fn parse_response(packet: &[u8]) -> Result<DnsAnswer, WireError> {
    let mut answer = DnsAnswer::default();
    let mut best_ech_priority: Option<u16> = None;

    parse_records(packet, |rtype, rclass, ttl, rdata| {
        if rclass != CLASS_IN {
            return;
        }
        match rtype {
            TYPE_A if rdata.len() == 4 => {
                answer.addrs.push(IpAddr::V4(Ipv4Addr::new(
                    rdata[0], rdata[1], rdata[2], rdata[3],
                )));
                update_min_ttl(&mut answer.min_ttl, ttl);
            }
            TYPE_AAAA if rdata.len() == 16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(rdata);
                answer.addrs.push(IpAddr::V6(Ipv6Addr::from(octets)));
                update_min_ttl(&mut answer.min_ttl, ttl);
            }
            TYPE_HTTPS | TYPE_SVCB => {
                if let Some((priority, ech)) = extract_ech_from_svcb(rdata)
                    && best_ech_priority.is_none_or(|cur| priority < cur)
                {
                    best_ech_priority = Some(priority);
                    answer.ech_config_list = Some(ech.to_vec());
                    update_min_ttl(&mut answer.min_ttl, ttl);
                }
            }
            _ => {}
        }
    })?;

    Ok(answer)
}

#[cfg(kani)]
mod proofs;

#[cfg(test)]
mod tests {
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
}
