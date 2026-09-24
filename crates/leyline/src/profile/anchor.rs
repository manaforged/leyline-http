#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HeaderAnchor {
    AfterCchUa,
    AfterCchUaMobile,
    AfterCchUaPlatform,
    AfterUserAgent,
    AfterAccept,
    AfterContentType,
    BeforeAcceptEncoding,
}

impl HeaderAnchor {
    pub(crate) fn anchor_name(&self) -> &'static str {
        match self {
            Self::AfterCchUa => "sec-ch-ua",
            Self::AfterCchUaMobile => "sec-ch-ua-mobile",
            Self::AfterCchUaPlatform => "sec-ch-ua-platform",
            Self::AfterUserAgent => "user-agent",
            Self::AfterAccept => "accept",
            Self::AfterContentType => "content-type",
            Self::BeforeAcceptEncoding => "accept-encoding",
        }
    }

    pub(crate) fn is_before(&self) -> bool {
        matches!(self, Self::BeforeAcceptEncoding)
    }
}

pub(crate) fn infer_anchor(name: &str) -> Option<HeaderAnchor> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "origin" => Some(HeaderAnchor::AfterContentType),

        "authorization" | "x-requested-with" | "x-csrf-token" | "x-requested-by" | "x-api-key" => {
            Some(HeaderAnchor::AfterUserAgent)
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests;
