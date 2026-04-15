//! Request presets — Chrome-accurate header templates for each request type.

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
pub type HeaderPair = (String, String);

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
        }
    }

    fn navigate_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            ("sec-ch-ua-platform".into(), format!("\"{}\"", ctx.sec_ch_ua_platform)),
            ("upgrade-insecure-requests".into(), "1".to_string()),
            ("user-agent".into(), ctx.user_agent.to_string()),
            ("accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".to_string()),
            ("sec-fetch-site".into(), "none".to_string()),
            ("sec-fetch-mode".into(), "navigate".to_string()),
            ("sec-fetch-user".into(), "?1".to_string()),
            ("sec-fetch-dest".into(), "document".to_string()),
            ("accept-encoding".into(), "gzip, deflate, br, zstd".to_string()),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }

    fn script_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            (
                "sec-ch-ua-platform".into(),
                format!("\"{}\"", ctx.sec_ch_ua_platform),
            ),
            ("user-agent".into(), ctx.user_agent.to_string()),
            ("accept".into(), "*/*".to_string()),
            ("sec-fetch-site".into(), "same-origin".to_string()),
            ("sec-fetch-mode".into(), "no-cors".to_string()),
            ("sec-fetch-dest".into(), "script".to_string()),
            ("referer".into(), ctx.referer.to_string()),
            (
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".to_string(),
            ),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }

    fn xhr_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            (
                "sec-ch-ua-platform".into(),
                format!("\"{}\"", ctx.sec_ch_ua_platform),
            ),
            ("user-agent".into(), ctx.user_agent.to_string()),
            (
                "accept".into(),
                "application/json, text/plain, */*".to_string(),
            ),
            ("origin".into(), ctx.origin.to_string()),
            ("sec-fetch-site".into(), "same-origin".to_string()),
            ("sec-fetch-mode".into(), "cors".to_string()),
            ("sec-fetch-dest".into(), "empty".to_string()),
            ("referer".into(), ctx.referer.to_string()),
            (
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".to_string(),
            ),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }

    fn form_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            (
                "sec-ch-ua-platform".into(),
                format!("\"{}\"", ctx.sec_ch_ua_platform),
            ),
            ("user-agent".into(), ctx.user_agent.to_string()),
            (
                "accept".into(),
                "application/json, text/plain, */*".to_string(),
            ),
            (
                "content-type".into(),
                "application/x-www-form-urlencoded".to_string(),
            ),
            ("origin".into(), ctx.origin.to_string()),
            ("sec-fetch-site".into(), "same-origin".to_string()),
            ("sec-fetch-mode".into(), "cors".to_string()),
            ("sec-fetch-dest".into(), "empty".to_string()),
            ("referer".into(), ctx.referer.to_string()),
            (
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".to_string(),
            ),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }

    fn cross_origin_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            (
                "sec-ch-ua-platform".into(),
                format!("\"{}\"", ctx.sec_ch_ua_platform),
            ),
            ("user-agent".into(), ctx.user_agent.to_string()),
            (
                "accept".into(),
                "application/json, text/plain, */*".to_string(),
            ),
            ("origin".into(), ctx.origin.to_string()),
            ("sec-fetch-site".into(), "cross-site".to_string()),
            ("sec-fetch-mode".into(), "cors".to_string()),
            ("sec-fetch-dest".into(), "empty".to_string()),
            ("referer".into(), ctx.referer.to_string()),
            (
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".to_string(),
            ),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }

    fn same_site_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        vec![
            ("sec-ch-ua".into(), ctx.sec_ch_ua.to_string()),
            ("sec-ch-ua-mobile".into(), ctx.sec_ch_ua_mobile.to_string()),
            (
                "sec-ch-ua-platform".into(),
                format!("\"{}\"", ctx.sec_ch_ua_platform),
            ),
            ("user-agent".into(), ctx.user_agent.to_string()),
            (
                "accept".into(),
                "application/json, text/plain, */*".to_string(),
            ),
            ("origin".into(), ctx.origin.to_string()),
            ("sec-fetch-site".into(), "same-site".to_string()),
            ("sec-fetch-mode".into(), "cors".to_string()),
            ("sec-fetch-dest".into(), "empty".to_string()),
            ("referer".into(), ctx.referer.to_string()),
            (
                "accept-encoding".into(),
                "gzip, deflate, br, zstd".to_string(),
            ),
            ("accept-language".into(), ctx.accept_language.to_string()),
        ]
    }
}
