//! Header anchors for caller-controlled positional injection.
//!
//! Chrome emits headers from three sources — Chrome-native, JS
//! `setRequestHeader`, and Chrome finalizers — interleaved at
//! well-defined positions. A caller that wants to emit a header at a
//! specific slot (for example `x-extra-6` that
//! must appear right after `user-agent`) names the slot with a `HeaderAnchor`.
//!
//! For well-known Chrome headers (`origin`, `x-requested-with`,
//! `x-csrf-token`, ...) the profile already knows the anchor, so
//! callers do not need to pass one — see [`infer_anchor`]. The
//! explicit anchor API is the escape hatch for site-specific headers
//! where no universal rule exists.
//!
//! ## Example
//!
//! A site can emit seven headers at five different
//! anchors in a site's XHR order:
//!
//! ```text
//! sec-ch-ua
//!   x-extra-1              <- AfterCchUa
//! sec-ch-ua-mobile
//!   x-extra-2             <- AfterCchUaMobile
//!   x-extra-3              <- AfterCchUaMobile
//!   x-extra-4              <- AfterCchUaMobile
//! sec-ch-ua-platform
//!   x-extra-5              <- AfterCchUaPlatform
//! user-agent
//!   x-extra-6              <- AfterUserAgent
//! accept
//! content-type
//!   x-extra-7              <- AfterContentType
//! origin
//! ...
//! ```
//!
//! None of those names have a universal Chrome rule; the profile
//! cannot possibly infer their positions. The caller supplies them
//! via `.anchored(anchor, name, value)` on the request builder.

/// Slot relative to a well-known Chrome header name.
///
/// Anchors are declarative — they name a slot, not a byte offset —
/// so the same anchor works across Navigate / Xhr / Form / etc.
/// presets as long as the named anchor header is present. If the
/// anchor header is absent for a given preset, the anchored header
/// is appended at the end.
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

    /// True when the anchor inserts *before* the named header; false
    /// when it inserts *after*.
    pub fn is_before(&self) -> bool {
        matches!(self, Self::BeforeAcceptEncoding)
    }
}

/// Default anchor for well-known Chrome request headers.
///
/// Returns `Some` for headers with a universal Chrome rule (so the
/// caller just writes `.header("origin", v)` and the profile handles
/// placement). Returns `None` for headers the profile cannot infer;
/// callers must use `.anchored(anchor, name, value)` for those.
///
/// Headers the preset already emits (`accept`, `user-agent`,
/// `sec-ch-ua*`, `accept-language`, `accept-encoding`, `referer`,
/// `sec-fetch-*`) return `None` — setting one via `.header()` on the
/// request builder replaces the preset value in-place rather than
/// re-anchoring.
pub fn infer_anchor(name: &str) -> Option<HeaderAnchor> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // Origin lands right after content-type on XHR/Form; on
        // presets without content-type (Xhr, CrossOrigin, SameSite)
        // the anchor falls through and the header is appended —
        // which also matches Chrome because those presets emit
        // `origin` natively already.
        "origin" => Some(HeaderAnchor::AfterContentType),

        // Authorization / CSRF / XHR markers ride after user-agent
        // in every Chrome XHR capture we have on file.
        "authorization" | "x-requested-with" | "x-csrf-token" | "x-requested-by" | "x-api-key" => {
            Some(HeaderAnchor::AfterUserAgent)
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_names_match_chrome_slots() {
        assert_eq!(HeaderAnchor::AfterCchUa.anchor_name(), "sec-ch-ua");
        assert_eq!(
            HeaderAnchor::AfterCchUaPlatform.anchor_name(),
            "sec-ch-ua-platform"
        );
        assert_eq!(HeaderAnchor::AfterUserAgent.anchor_name(), "user-agent");
        assert_eq!(
            HeaderAnchor::BeforeAcceptEncoding.anchor_name(),
            "accept-encoding"
        );
    }

    #[test]
    fn before_anchor_flagged() {
        assert!(HeaderAnchor::BeforeAcceptEncoding.is_before());
        assert!(!HeaderAnchor::AfterCchUa.is_before());
    }

    #[test]
    fn infer_anchor_well_known_headers() {
        assert_eq!(infer_anchor("origin"), Some(HeaderAnchor::AfterContentType));
        assert_eq!(infer_anchor("Origin"), Some(HeaderAnchor::AfterContentType));
        assert_eq!(
            infer_anchor("authorization"),
            Some(HeaderAnchor::AfterUserAgent)
        );
        assert_eq!(
            infer_anchor("X-Csrf-Token"),
            Some(HeaderAnchor::AfterUserAgent)
        );
        assert_eq!(
            infer_anchor("x-requested-with"),
            Some(HeaderAnchor::AfterUserAgent)
        );
    }

    #[test]
    fn infer_anchor_none_for_custom_headers() {
        assert_eq!(infer_anchor("x-extra-6"), None);
        assert_eq!(infer_anchor("x-vendor-whatever"), None);
        assert_eq!(infer_anchor("x-custom"), None);
    }

    #[test]
    fn infer_anchor_none_for_preset_owned_headers() {
        // Preset already emits these; setting them via .header()
        // replaces the preset value rather than re-anchoring.
        assert_eq!(infer_anchor("accept"), None);
        assert_eq!(infer_anchor("user-agent"), None);
        assert_eq!(infer_anchor("accept-language"), None);
    }
}
