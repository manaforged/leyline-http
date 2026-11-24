/// Names where an extra header goes, relative to an anchor header in the profile's header order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HeaderPosition {
    /// Inserts the header before `sec-ch-ua`.
    BeforeSecChUa,
    /// Inserts the header after `sec-ch-ua`.
    AfterSecChUa,
    /// Inserts the header after `sec-ch-ua-mobile`.
    AfterSecChUaMobile,
    /// Inserts the header after `sec-ch-ua-platform`.
    AfterSecChUaPlatform,
    /// Inserts the header after `user-agent`.
    AfterUserAgent,
    /// Inserts the header after `accept`.
    AfterAccept,
    /// Inserts the header after `content-type`.
    AfterContentType,
    /// Inserts the header after `sec-fetch-dest`.
    AfterSecFetchDest,
    /// Inserts the header before `accept-encoding`.
    BeforeAcceptEncoding,
}

impl HeaderPosition {
    pub(crate) fn anchor_name(&self) -> &'static str {
        match self {
            Self::BeforeSecChUa | Self::AfterSecChUa => "sec-ch-ua",
            Self::AfterSecChUaMobile => "sec-ch-ua-mobile",
            Self::AfterSecChUaPlatform => "sec-ch-ua-platform",
            Self::AfterUserAgent => "user-agent",
            Self::AfterAccept => "accept",
            Self::AfterContentType => "content-type",
            Self::AfterSecFetchDest => "sec-fetch-dest",
            Self::BeforeAcceptEncoding => "accept-encoding",
        }
    }

    pub(crate) fn is_before(&self) -> bool {
        matches!(self, Self::BeforeSecChUa | Self::BeforeAcceptEncoding)
    }
}

pub(crate) fn infer_anchor(name: &str) -> Option<HeaderPosition> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "origin" => Some(HeaderPosition::AfterContentType),

        "authorization" | "x-requested-with" | "x-csrf-token" | "x-requested-by" | "x-api-key" => {
            Some(HeaderPosition::AfterUserAgent)
        }

        _ => None,
    }
}
