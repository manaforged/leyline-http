//! Header anchors for caller-controlled positional injection.

/// Slot relative to a well-known Chrome header name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HeaderAnchor {
    /// Insert immediately after `sec-ch-ua`.
    AfterCchUa,
    /// Insert immediately after `sec-ch-ua-mobile`.
    AfterCchUaMobile,
    /// Insert immediately after `sec-ch-ua-platform`.
    AfterCchUaPlatform,
    /// Insert immediately after `user-agent`.
    AfterUserAgent,
    /// Insert immediately after `accept`.
    AfterAccept,
    /// Insert immediately after `content-type`.
    AfterContentType,
    /// Insert immediately before `accept-encoding`.
    BeforeAcceptEncoding,
}

impl HeaderAnchor {
    /// Header name that identifies this anchor's position.
    pub fn anchor_name(&self) -> &'static str {
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

    /// True when the anchor inserts *before* the named header; false when it inserts *after*.
    pub fn is_before(&self) -> bool {
        matches!(self, Self::BeforeAcceptEncoding)
    }
}

/// Default anchor for well-known Chrome request headers.
pub fn infer_anchor(name: &str) -> Option<HeaderAnchor> {
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
