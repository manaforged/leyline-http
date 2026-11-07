use crate::error::BodyLimit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overflow {
    Fail,
    Truncate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cap {
    pub(crate) limit: BodyLimit,
    pub(crate) overflow: Overflow,
}

impl Cap {
    pub(crate) fn fail(limit: BodyLimit) -> Self {
        Self {
            limit,
            overflow: Overflow::Fail,
        }
    }

    pub(crate) fn truncate_at(session: BodyLimit, caller: usize) -> Self {
        Self {
            limit: session.at_most(caller),
            overflow: Overflow::Truncate,
        }
    }

    pub(crate) fn room(self, used: usize) -> usize {
        self.limit.room(used)
    }
}
