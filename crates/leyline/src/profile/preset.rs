//! Request presets — Chrome-accurate header templates for each request type.

use std::borrow::Cow;

/// Request type preset that determines sec-fetch-* headers and ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Preset {
    /// Page navigation (GET document).
    Navigate,
    /// Script/CSS resource fetch.
    Script,
    /// XHR/fetch JSON API call.
    Xhr,
    /// Form POST.
    Form,
    /// Cross-origin API call.
    CrossOrigin,
    /// Same-site subdomain API call.
    SameSite,
    /// Form-submit POST whose response is a top-level *document* navigation
    /// (e.g. classic `<form action="..." method="post">` with no `fetch()`
    /// wrapper — clicking the form button replaces the page). Distinct
    /// from `Form` (XHR-shaped CORS POST) and `Navigate` (top-level GET).
    /// Captured against Chrome 147 form-POST navigations.
    FormNavigate,
}

/// Context needed to build preset headers.
pub struct HeaderContext<'a> {
    /// Full `User-Agent` string.
    pub user_agent: &'a str,
    /// `Sec-CH-UA` brand list, already quoted per Chrome's format.
    pub sec_ch_ua: &'a str,
    /// `Sec-CH-UA-Mobile` flag (`?0` for desktop, `?1` for mobile).
    pub sec_ch_ua_mobile: &'a str,
    /// Platform label used in `Sec-CH-UA-Platform` (quoted downstream).
    pub sec_ch_ua_platform: &'a str,
    /// `Accept-Language` header value.
    pub accept_language: &'a str,
    /// Origin of the current request (`scheme://host[:port]`).
    pub origin: &'a str,
    /// `Referer` header value, or empty if none.
    pub referer: &'a str,
}

/// A single header name-value pair, in insertion order.
///
/// Both parts are `Cow<'static, str>`: header names and Chrome-constant values
/// are `Borrowed` static literals (zero allocation), while session- or
/// request-derived values (UA, origin, referer) are `Owned`. The `'static`
/// bound lets the assembled list move into the H2 driver's command channel.
pub type HeaderPair = (Cow<'static, str>, Cow<'static, str>);

/// Borrowed (zero-alloc) header part from a static literal.
#[inline]
fn b(s: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(s)
}

/// Owned header part from a runtime string (the session UA, an origin, …).
#[inline]
fn o(s: &str) -> Cow<'static, str> {
    Cow::Owned(s.to_string())
}

impl Preset {
    /// Build the ordered header list for this preset.
    pub fn build_headers(&self, ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        match self {
            Self::Navigate => Self::navigate_headers(ctx),
            Self::Script => Self::script_headers(ctx),
            Self::Xhr => Self::xhr_headers(ctx),
            Self::Form => Self::form_headers(ctx),
            Self::CrossOrigin => Self::cross_origin_headers(ctx),
            Self::SameSite => Self::same_site_headers(ctx),
            Self::FormNavigate => Self::form_navigate_headers(ctx),
        }
    }

    /// `sec-ch-ua-platform`, quoted per Chrome's wire format. Shared by
    /// every preset so the quoting lives in exactly one place.
    fn sec_ch_ua_platform(ctx: &HeaderContext<'_>) -> HeaderPair {
        (
            b("sec-ch-ua-platform"),
            Cow::Owned(format!("\"{}\"", ctx.sec_ch_ua_platform)),
        )
    }

    /// `accept-encoding` value shared by every Chrome-shaped preset.
    fn accept_encoding() -> HeaderPair {
        (b("accept-encoding"), b("gzip, deflate, br, zstd"))
    }

    fn navigate_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("upgrade-insecure-requests"), b("1")),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7")),
            (b("sec-fetch-site"), b("none")),
            (b("sec-fetch-mode"), b("navigate")),
            (b("sec-fetch-user"), b("?1")),
            (b("sec-fetch-dest"), b("document")),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn script_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("*/*")),
            (b("sec-fetch-site"), b("same-origin")),
            (b("sec-fetch-mode"), b("no-cors")),
            (b("sec-fetch-dest"), b("script")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn xhr_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-origin")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn form_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("content-type"), b("application/x-www-form-urlencoded")),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-origin")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn cross_origin_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("cross-site")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn form_navigate_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        // Form-submit-as-top-level-navigation: classic `<form method=post>`
        // where the response replaces the document. Chrome's wire shape
        // mirrors `Navigate` (full text/html accept block, sec-fetch-mode
        // navigate, sec-fetch-dest document, sec-fetch-user ?1, upgrade-
        // insecure-requests) plus the POST-specific cache-control,
        // content-type, and origin headers. sec-fetch-site is `same-origin`
        // because a form button click is always referred from a page on
        // the form's own origin.
        vec![
            (b("cache-control"), b("max-age=0")),
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("upgrade-insecure-requests"), b("1")),
            (b("user-agent"), o(ctx.user_agent)),
            (b("content-type"), b("application/x-www-form-urlencoded")),
            (b("accept"), b("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7")),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-origin")),
            (b("sec-fetch-mode"), b("navigate")),
            (b("sec-fetch-user"), b("?1")),
            (b("sec-fetch-dest"), b("document")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }

    fn same_site_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-site")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("accept-language"), o(ctx.accept_language)),
        ]
    }
}
