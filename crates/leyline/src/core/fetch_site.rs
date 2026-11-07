use url::Url;

use crate::core::config::Origin;
use crate::util::registrable_domain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub(crate) enum FetchSite {
    None,
    SameOrigin,
    SameSite,
    CrossSite,
}

impl FetchSite {
    #[must_use]
    pub(crate) fn of(context: &Url, request: &Url) -> Self {
        if Origin::same(context, request) {
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

    pub(crate) fn across<'a>(context: &Url, chain: impl IntoIterator<Item = &'a Url>) -> Self {
        chain
            .into_iter()
            .map(|url| Self::of(context, url))
            .max_by_key(|site| match site {
                Self::None | Self::SameOrigin => 0,
                Self::SameSite => 1,
                Self::CrossSite => 2,
            })
            .unwrap_or(Self::SameOrigin)
    }

    const ALL: [Self; 4] = [
        Self::None,
        Self::SameOrigin,
        Self::SameSite,
        Self::CrossSite,
    ];

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|site| value.eq_ignore_ascii_case(site.as_str()))
    }

    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
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

#[cfg(test)]
#[path = "fetch_site_tests.rs"]
mod tests;
