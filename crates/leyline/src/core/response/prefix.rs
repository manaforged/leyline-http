#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StopReason {
    PredicateMatched,
    LimitReached,
    EndOfBody,
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
