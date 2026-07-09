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
    /// Firefox (Gecko) identity. The presets are Chrome-shaped, so when this is set
    /// [`Preset::build_headers`] strips the Chrome-only `Sec-CH-UA*` Client Hints (Firefox
    /// emits none) and swaps the Chrome document `Accept` for the Gecko one.
    pub firefox: bool,
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

/// Firefox's document `Accept` (Gecko) — captured live from tls.peet.ws (Firefox 153, Windows):
/// `text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`, with none of Chrome's
/// `image/apng` / `application/signed-exchange` / image types. Swapped in for the Chrome document
/// Accept on a Firefox identity so the request agrees with the Firefox JA4.
const FIREFOX_DOC_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

/// Real Firefox H2 request-header order, captured live from tls.peet.ws (Firefox 153, Windows). The
/// navigate sequence (`user-agent … sec-fetch-user, priority, te`) is capture-exact; the subresource
/// headers (`content-type`/`origin`/`referer`/`cookie`) are placed at Firefox-conventional positions
/// pending a cookied-XHR capture. Applied by the session AFTER the full header set is assembled
/// (including `cookie`) so every Firefox request matches this order.
pub(crate) const FIREFOX_HEADER_ORDER: &[&str] = &[
    "user-agent",
    "accept",
    "accept-language",
    "accept-encoding",
    "content-type",
    "upgrade-insecure-requests",
    "origin",
    "referer",
    "cookie",
    "sec-fetch-dest",
    "sec-fetch-mode",
    "sec-fetch-site",
    "sec-fetch-user",
    "priority",
    "te",
];

impl Preset {
    /// Build the ordered header list for this preset. The presets are Chrome-shaped; a Firefox
    /// identity ([`HeaderContext::firefox`]) is reshaped to drop the Chrome-only Client Hints and
    /// use the Gecko document `Accept` so the request layer agrees with the Firefox TLS/JA4.
    pub fn build_headers(&self, ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        let mut headers = match self {
            Self::Navigate => Self::navigate_headers(ctx),
            Self::Script => Self::script_headers(ctx),
            Self::Xhr => Self::xhr_headers(ctx),
            Self::Form => Self::form_headers(ctx),
            Self::CrossOrigin => Self::cross_origin_headers(ctx),
            Self::SameSite => Self::same_site_headers(ctx),
            Self::FormNavigate => Self::form_navigate_headers(ctx),
        };
        if ctx.firefox {
            Self::reshape_for_firefox(*self, &mut headers);
        }
        headers
    }

    /// Reshape a Chrome-shaped preset to Firefox's request SET (the session applies Firefox's header
    /// ORDER afterward, via [`FIREFOX_HEADER_ORDER`]). Firefox emits NO `Sec-CH-UA*` Client Hints
    /// (a Chromium feature), uses a Gecko document `Accept`, and carries an HTTP `priority` hint plus
    /// `te: trailers` on every H2 request — all captured live from tls.peet.ws (Firefox 153).
    fn reshape_for_firefox(preset: Preset, headers: &mut Vec<HeaderPair>) {
        headers.retain(|(name, _)| !name.starts_with("sec-ch-ua"));
        // Only the document (`text/html…`) Accept is browser-specific; the XHR/script Accepts
        // (`*/*`, `application/json…`) are JS-set and browser-neutral, so leave them.
        for (name, value) in headers.iter_mut() {
            if name == "accept" && value.starts_with("text/html") {
                *value = Cow::Borrowed(FIREFOX_DOC_ACCEPT);
            }
        }
        // Document loads carry `priority: u=0, i` (capture-verified); subresource/API requests use
        // `u=1, i` (Firefox-conventional). `te: trailers` rides every Firefox H2 request.
        let priority = if matches!(preset, Preset::Navigate | Preset::FormNavigate) {
            "u=0, i"
        } else {
            "u=1, i"
        };
        headers.push((b("priority"), b(priority)));
        headers.push((b("te"), b("trailers")));
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
            (
                b("accept"),
                b(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
                ),
            ),
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
            (
                b("accept"),
                b(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
                ),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(firefox: bool) -> HeaderContext<'static> {
        HeaderContext {
            user_agent: "UA",
            sec_ch_ua: "\"Chromium\";v=\"148\"",
            sec_ch_ua_mobile: "?0",
            sec_ch_ua_platform: "Windows",
            accept_language: "en-US,en;q=0.9",
            origin: "https://x.com",
            referer: "https://x.com/",
            firefox,
        }
    }

    fn names(h: &[HeaderPair]) -> Vec<String> {
        h.iter().map(|(n, _)| n.to_string()).collect()
    }

    fn value<'a>(h: &'a [HeaderPair], name: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_ref())
    }

    /// A Firefox identity must ship NO `sec-ch-ua*` Client Hints (Firefox emits none), the Gecko
    /// document `Accept`, and `te: trailers` + an HTTP `priority` hint; a Chrome identity keeps the
    /// Client Hints and sends neither. Values are from a live Firefox 153 tls.peet.ws capture.
    #[test]
    fn firefox_reshapes_client_hints_accept_priority_and_te() {
        // Chrome identity: CH headers present + Chrome document Accept, no priority/te.
        let chrome = Preset::Navigate.build_headers(&ctx(false));
        assert!(names(&chrome).iter().any(|n| n == "sec-ch-ua-mobile"));
        assert!(value(&chrome, "accept").unwrap().contains("image/apng"));
        assert_eq!(value(&chrome, "priority"), None);
        assert_eq!(value(&chrome, "te"), None);

        // Firefox navigate: zero Client Hints, Gecko document Accept, priority u=0, te trailers.
        let ff = Preset::Navigate.build_headers(&ctx(true));
        assert!(
            names(&ff).iter().all(|n| !n.starts_with("sec-ch-ua")),
            "firefox must send no Client Hints: {:?}",
            names(&ff)
        );
        assert_eq!(value(&ff, "accept"), Some(FIREFOX_DOC_ACCEPT));
        assert_eq!(value(&ff, "priority"), Some("u=0, i"));
        assert_eq!(value(&ff, "te"), Some("trailers"));

        // Firefox XHR/API: browser-neutral JS-set Accept untouched, priority u=1, te trailers.
        let ff_xhr = Preset::SameSite.build_headers(&ctx(true));
        assert!(names(&ff_xhr).iter().all(|n| !n.starts_with("sec-ch-ua")));
        assert_eq!(
            value(&ff_xhr, "accept"),
            Some("application/json, text/plain, */*")
        );
        assert_eq!(value(&ff_xhr, "priority"), Some("u=1, i"));
        assert_eq!(value(&ff_xhr, "te"), Some("trailers"));
    }
}
