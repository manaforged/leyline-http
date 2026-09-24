use std::sync::Arc;

#[derive(Clone)]
#[non_exhaustive]
pub struct RedirectPolicy {
    kind: RedirectKind,
}

type RedirectFn = Arc<dyn Fn(RedirectAttempt<'_>) -> RedirectAction + Send + Sync>;

#[derive(Clone)]
enum RedirectKind {
    Limited(usize),
    None,
    Custom(RedirectFn),
}

impl std::fmt::Debug for RedirectPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            RedirectKind::Limited(n) => f.debug_tuple("RedirectPolicy::Limited").field(n).finish(),
            RedirectKind::None => f.write_str("RedirectPolicy::None"),
            RedirectKind::Custom(_) => f.write_str("RedirectPolicy::Custom(..)"),
        }
    }
}

impl Default for RedirectPolicy {
    fn default() -> Self {
        Self::limited(10)
    }
}

impl RedirectPolicy {
    pub fn limited(max: usize) -> Self {
        Self {
            kind: RedirectKind::Limited(max),
        }
    }

    pub fn none() -> Self {
        Self {
            kind: RedirectKind::None,
        }
    }

    pub fn custom<F>(f: F) -> Self
    where
        F: Fn(RedirectAttempt<'_>) -> RedirectAction + Send + Sync + 'static,
    {
        Self {
            kind: RedirectKind::Custom(Arc::new(f)),
        }
    }

    pub(crate) fn max_redirects_hint(&self) -> usize {
        match self.kind {
            RedirectKind::Limited(n) => n,
            RedirectKind::None => 0,
            RedirectKind::Custom(_) => 32,
        }
    }

    pub(crate) fn action(&self, attempt: RedirectAttempt<'_>) -> RedirectAction {
        match &self.kind {
            RedirectKind::Limited(max) => {
                if attempt.previous.len() < *max {
                    RedirectAction::Follow
                } else {
                    RedirectAction::Stop
                }
            }
            RedirectKind::None => RedirectAction::Stop,
            RedirectKind::Custom(f) => f(attempt),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RedirectAttempt<'a> {
    pub status: u16,
    pub url: &'a http::Uri,
    pub location: Option<&'a str>,
    pub previous: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectAction {
    Follow,
    Stop,
}
