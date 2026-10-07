#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopReason {
    PredicateMatched,
    LimitReached,
    EndOfBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrefixRead {
    pub bytes: Vec<u8>,
    pub stopped_by: StopReason,
}

pub(super) fn stop_reason<F>(
    out: &[u8],
    from: usize,
    limit: usize,
    done: &mut F,
) -> Option<StopReason>
where
    F: FnMut(&[u8], usize) -> bool,
{
    if out.len() > from && done(out, from) {
        return Some(StopReason::PredicateMatched);
    }
    (out.len() >= limit).then_some(StopReason::LimitReached)
}
