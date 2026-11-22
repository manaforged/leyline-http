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

pub(crate) fn encode_query(host: &str, qtype: u16) -> Result<Vec<u8>, WireError> {
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    if trimmed.is_empty() {
        return Err(WireError::InvalidName);
    }
    let mut packet = Vec::with_capacity(HEADER_LEN + trimmed.len() + 6);
    packet.extend_from_slice(&[
        0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    let name_start = packet.len();
    for label in trimmed.split('.') {
        if label.is_empty() || label.len() > MAX_LABEL_LEN || !label.is_ascii() {
            return Err(WireError::InvalidName);
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    if packet.len() - name_start > MAX_NAME_LEN {
        return Err(WireError::InvalidName);
    }
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(packet)
}

pub(crate) fn skip_name(packet: &[u8], start: usize) -> Option<usize> {
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
                if target >= ceiling || target >= packet.len() || jumps >= MAX_JUMPS {
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

pub(crate) fn parse_response(packet: &[u8]) -> Result<DnsAnswer, WireError> {
    let (qdcount, ancount) = parse_header(packet)?;

    let mut pos = HEADER_LEN;
    for _ in 0..qdcount {
        pos = skip_name(packet, pos).ok_or(WireError::MalformedName)?;
        pos = pos.checked_add(4).ok_or(WireError::Truncated)?;
        if pos > packet.len() {
            return Err(WireError::Truncated);
        }
    }

    let mut answer = DnsAnswer::default();
    let mut best_ech_priority: Option<u16> = None;

    for _ in 0..ancount {
        pos = skip_name(packet, pos).ok_or(WireError::MalformedName)?;
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
        pos = rdata_end;

        if rclass != CLASS_IN {
            continue;
        }
        match rtype {
            TYPE_A if rdata.len() == 4 => {
                answer.addrs.push(IpAddr::V4(Ipv4Addr::new(
                    rdata[0], rdata[1], rdata[2], rdata[3],
                )));
                answer.min_ttl = Some(answer.min_ttl.map_or(ttl, |cur| cur.min(ttl)));
            }
            TYPE_AAAA if rdata.len() == 16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(rdata);
                answer.addrs.push(IpAddr::V6(Ipv6Addr::from(octets)));
                answer.min_ttl = Some(answer.min_ttl.map_or(ttl, |cur| cur.min(ttl)));
            }
            TYPE_HTTPS | TYPE_SVCB => {
                if let Some((priority, ech)) = extract_ech_from_svcb(rdata)
                    && best_ech_priority.is_none_or(|cur| priority < cur)
                {
                    best_ech_priority = Some(priority);
                    answer.ech_config_list = Some(ech.to_vec());
                    answer.min_ttl = Some(answer.min_ttl.map_or(ttl, |cur| cur.min(ttl)));
                }
            }
            _ => {}
        }
    }

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
}
