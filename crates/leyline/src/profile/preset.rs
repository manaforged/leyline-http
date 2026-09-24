use std::borrow::Cow;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum HeaderStyle {
    #[default]
    Chromium,
    Gecko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Preset {
    Native,
    Navigate,
    Script,
    Xhr,
    Form,
    CrossOrigin,
    SameSite,
    FormNavigate,
}

pub struct HeaderContext<'a> {
    pub user_agent: &'a str,
    pub sec_ch_ua: &'a str,
    pub sec_ch_ua_mobile: &'a str,
    pub sec_ch_ua_platform: &'a str,
    pub accept_language: &'a str,
    pub origin: &'a str,
    pub referer: &'a str,
    pub firefox: bool,
}

pub type HeaderPair = (Cow<'static, str>, Cow<'static, str>);

#[inline]
fn b(s: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(s)
}

#[inline]
fn o(s: &str) -> Cow<'static, str> {
    Cow::Owned(s.to_string())
}

const FIREFOX_DOC_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

impl Preset {
    pub(crate) fn build_headers(&self, ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
        let mut headers = match self {
            Self::Native => vec![
                (b("user-agent"), o(ctx.user_agent)),
                (b("accept"), b("*/*")),
                (b("accept-encoding"), b("gzip, deflate, br, zstd")),
                (b("accept-language"), o(ctx.accept_language)),
            ],
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

    fn reshape_for_firefox(preset: Preset, headers: &mut Vec<HeaderPair>) {
        headers.retain(|(name, _)| !name.starts_with("sec-ch-ua"));
        for (name, value) in headers.iter_mut() {
            if name == "accept" && value.starts_with("text/html") {
                *value = Cow::Borrowed(FIREFOX_DOC_ACCEPT);
            }
        }
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("priority"));
        let priority = if matches!(preset, Preset::Navigate | Preset::FormNavigate) {
            "u=0, i"
        } else {
            "u=1, i"
        };
        headers.push((b("priority"), b(priority)));
        headers.push((b("te"), b("trailers")));
    }

    fn sec_ch_ua_platform(ctx: &HeaderContext<'_>) -> HeaderPair {
        (
            b("sec-ch-ua-platform"),
            Cow::Owned(format!("\"{}\"", ctx.sec_ch_ua_platform)),
        )
    }

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
            (b("accept-language"), o(ctx.accept_language)),
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
            (b("priority"), b("u=0, i")),
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
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("accept-language"), o(ctx.accept_language)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-origin")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("priority"), b("u=1, i")),
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
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("accept-language"), o(ctx.accept_language)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("cross-site")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("priority"), b("u=1, i")),
        ]
    }

    fn form_navigate_headers(ctx: &HeaderContext<'_>) -> Vec<HeaderPair> {
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
            Self::sec_ch_ua_platform(ctx),
            (b("user-agent"), o(ctx.user_agent)),
            (b("accept"), b("application/json, text/plain, */*")),
            (b("sec-ch-ua"), o(ctx.sec_ch_ua)),
            (b("accept-language"), o(ctx.accept_language)),
            (b("sec-ch-ua-mobile"), o(ctx.sec_ch_ua_mobile)),
            (b("origin"), o(ctx.origin)),
            (b("sec-fetch-site"), b("same-site")),
            (b("sec-fetch-mode"), b("cors")),
            (b("sec-fetch-dest"), b("empty")),
            (b("referer"), o(ctx.referer)),
            Self::accept_encoding(),
            (b("priority"), b("u=1, i")),
        ]
    }
}

#[cfg(test)]
mod tests;
