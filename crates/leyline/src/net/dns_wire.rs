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
mod tests;
