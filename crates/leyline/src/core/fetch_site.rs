use url::Url;

use crate::cookie::parse::registrable_domain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FetchSite {
    SameOrigin,
    SameSite,
    CrossSite,
}

impl FetchSite {
    #[must_use]
    pub fn of(context: &Url, request: &Url) -> Self {
        if context.origin() == request.origin() {
            return Self::SameOrigin;
        }
        let same_site = context.scheme() == request.scheme()
            && match (context.host_str(), request.host_str()) {
                (Some(a), Some(b)) => match (registrable_domain(a), registrable_domain(b)) {
                    (Some(da), Some(db)) => da == db,
                    _ => a == b,
                },
                _ => false,
            };
        if same_site {
            Self::SameSite
        } else {
            Self::CrossSite
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameOrigin => "same-origin",
            Self::SameSite => "same-site",
            Self::CrossSite => "cross-site",
        }
    }
}

impl std::fmt::Display for FetchSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
