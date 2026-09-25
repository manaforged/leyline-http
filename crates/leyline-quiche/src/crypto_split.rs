use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitialCryptoSplit {
    #[default]
    Fill,
    Even,
    EvenSniSlice,
}

pub(crate) fn first_packet_chunks(
    split: InitialCryptoSplit,
    hello: &[u8],
    room: usize,
) -> Vec<Range<usize>> {
    let total = hello.len();
    let limit = total / total.div_ceil(room.max(1)).max(1);
    let sni = match split {
        InitialCryptoSplit::Fill => return vec![0..total.min(room)],
        InitialCryptoSplit::Even => None,
        InitialCryptoSplit::EvenSniSlice => find_sni(hello),
    };
    let Some(sni) = sni else {
        return vec![0..limit];
    };
    let mid = sni.start + (sni.end - sni.start) / 2;
    let (left, right) = slice_around(mid, total, limit);
    [right, left]
        .into_iter()
        .filter(|r| !r.is_empty())
        .collect()
}

fn slice_around(mid: usize, total: usize, limit: usize) -> (Range<usize>, Range<usize>) {
    let (left, right) = (mid, total - mid);
    if left + right <= limit {
        (0..mid, mid..total)
    } else if left <= limit {
        (0..mid, total - (limit - left)..total)
    } else if right <= limit {
        (0..limit - right, mid..total)
    } else {
        (0..limit / 2, mid..mid + limit / 2)
    }
}

pub(crate) fn unsent(chunks: &[Range<usize>], total: usize) -> Vec<Range<usize>> {
    let mut sorted = chunks.to_vec();
    sorted.sort_by_key(|r| r.start);
    let mut gaps = Vec::new();
    let mut cursor = 0;
    for chunk in sorted {
        if chunk.start > cursor {
            gaps.push(cursor..chunk.start);
        }
        cursor = cursor.max(chunk.end);
    }
    if cursor < total {
        gaps.push(cursor..total);
    }
    gaps
}

fn find_sni(hello: &[u8]) -> Option<Range<usize>> {
    let mut b = octets::Octets::with_slice(hello);
    if b.get_u8().ok()? != 1 {
        return None;
    }
    b.skip(3 + 2 + 32).ok()?;
    let session_id = b.get_u8().ok()?;
    b.skip(usize::from(session_id)).ok()?;
    let suites = b.get_u16().ok()?;
    b.skip(usize::from(suites)).ok()?;
    let compression = b.get_u8().ok()?;
    b.skip(usize::from(compression)).ok()?;
    b.skip(2).ok()?;
    while b.cap() >= 4 {
        let ext_type = b.get_u16().ok()?;
        let ext_len = b.get_u16().ok()?;
        if ext_type == 0 {
            let list_len = usize::from(b.get_u16().ok()?);
            b.skip(3).ok()?;
            let start = b.off();
            let end = start + list_len.checked_sub(3)?;
            return (end <= hello.len()).then_some(start..end);
        }
        b.skip(usize::from(ext_len)).ok()?;
    }
    None
}
