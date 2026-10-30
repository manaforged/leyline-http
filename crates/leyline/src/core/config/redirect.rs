use std::sync::Arc;

use crate::core::attempt::Attempt;

#[doc = include_str!("../../../docs/redirect.md")]
#[derive(Clone)]
#[non_exhaustive]
#[must_use]
pub struct Redirect {
    kind: RedirectKind,
}

type RedirectFn = Arc<dyn Fn(&Attempt<'_>) -> bool + Send + Sync>;

#[derive(Clone)]
enum RedirectKind {
    Limited(usize),
    None,
    Custom(RedirectFn),
}

impl std::fmt::Debug for Redirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            RedirectKind::Limited(n) => f.debug_tuple("Redirect::Limited").field(n).finish(),
            RedirectKind::None => f.write_str("Redirect::None"),
            RedirectKind::Custom(_) => f.write_str("Redirect::Custom(..)"),
        }
    }
}

impl From<usize> for Redirect {
    fn from(max: usize) -> Self {
        Self::limit(max)
    }
}

impl Default for Redirect {
    fn default() -> Self {
        Self::limit(10)
    }
}

impl Redirect {
    /// Follows at most `max` redirects; the default is 10.
    pub fn limit(max: usize) -> Self {
        Self {
            kind: RedirectKind::Limited(max),
        }
    }

    /// Follows no redirects.
    pub fn none() -> Self {
        Self {
            kind: RedirectKind::None,
        }
    }

    /// Follows a redirect when the closure returns true for its `Attempt`.
    pub fn custom<F>(f: F) -> Self
    where
        F: Fn(&Attempt<'_>) -> bool + Send + Sync + 'static,
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

    pub(crate) fn follows(&self, attempt: &Attempt<'_>) -> bool {
        match &self.kind {
            RedirectKind::Limited(max) => attempt.previous().len() < *max,
            RedirectKind::None => false,
            RedirectKind::Custom(f) => f(attempt),
        }
    }
}
