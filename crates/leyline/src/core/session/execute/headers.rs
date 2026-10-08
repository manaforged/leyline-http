use std::borrow::Cow;

use http::header::{AUTHORIZATION, HOST};
use url::Url;

use super::super::header_merge::apply_extra_headers;
use super::journey::Journey;
use super::{RequestContext, SiteContext, fetch_site_for, referer_for, url_origin};
use crate::FetchSite;
use crate::core::Session;
use crate::core::body::{Body, BodyKind};
use crate::core::headers::{HeaderList, reorder};
use crate::profile::Preset;
use crate::profile::preset::HeaderPair;
use crate::util::sensitive_header;

impl Session {
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is a distinct per-attempt input with no shared owner"
    )]
    pub(super) fn attempt_headers(
        &self,
        preset: Option<Preset>,
        request: RequestContext<'_>,
        current_url: &Url,
        current_method: &str,
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
        header_order: Option<&[String]>,
    ) -> Vec<HeaderPair> {
        let referer = request.referer;
        let ctx = self.header_context(&request);
        let mut headers = self.inner.header_style.build_headers(preset, &ctx);
        self.shape_bare(&mut headers);
        if matches!(current_body.0, BodyKind::Empty) {
            headers.retain(|(k, _)| !is_request_body_header(k));
        }
        shape_origin(&mut headers, current_method, request.origin_downgrade);
        let caller_has = |name: &str| {
            extra_headers
                .is_some_and(|h| h.iter().any(|(k, _)| k.as_str().eq_ignore_ascii_case(name)))
        };
        self.merge_session_headers(&mut headers, &caller_has, strip_sensitive, current_url);

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &cross_origin_stripped);
        }
        if request.initiated {
            add_referer(&mut headers, referer);
        }

        set_content_length(&mut headers, current_body);

        self.add_jar_cookie(&mut headers, current_url, &request.site, current_method);

        if let Some(order) = header_order
            .map(Cow::Borrowed)
            .or_else(|| self.session_header_order())
        {
            reorder(&mut headers, &order);
        }

        headers
    }

    fn header_context<'a>(
        &'a self,
        request: &RequestContext<'a>,
    ) -> crate::profile::preset::HeaderContext<'a> {
        let hints = !self.inner.sec_ch_ua.is_empty();
        let referer = request.referer;
        crate::profile::preset::HeaderContext {
            user_agent: &self.inner.user_agent,
            sec_ch_ua: &self.inner.sec_ch_ua,
            sec_ch_ua_mobile: if hints {
                self.inner.platform.mobile_flag()
            } else {
                ""
            },
            sec_ch_ua_platform: if hints {
                self.inner.platform.sec_ch_platform()
            } else {
                ""
            },
            accept_language: &self.inner.accept_language,
            origin: request.origin,
            referer,
            fetch_site: request.site.fetch_site.as_str(),
            navigation_site: if request.initiated {
                request.site.fetch_site.as_str()
            } else {
                "none"
            },
            navigation_referer: if request.initiated { referer } else { "" },
        }
    }

    fn shape_bare(&self, headers: &mut Vec<HeaderPair>) {
        if self.inner.header_style == crate::profile::HeaderStyle::Bare {
            advertise_codecs(headers, self.inner.compression.accept_encoding());
        }
    }

    pub(super) fn leg_headers(
        &self,
        journey: &Journey,
        preset: Option<Preset>,
        header_order: Option<&[String]>,
    ) -> Vec<HeaderPair> {
        let origin = url_origin(&journey.url);
        let referer = referer_for(Some(&journey.referrer), &origin);
        let fetch_site = fetch_site_for(journey.initiator.as_ref(), &journey.chain, &journey.url);
        let strip_sensitive = journey.strips_credentials(&origin);
        let initiator_origin = journey
            .initiator
            .as_ref()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .map(url_origin);
        let downgrade = initiator_origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("https:") && journey.url.scheme() != "https");
        let request_origin = match (&initiator_origin, journey.tainted) {
            (_, true) => "null",
            (Some(origin), false) => origin.as_str(),
            (None, false) => journey.original_origin.as_str(),
        };
        self.attempt_headers(
            preset,
            RequestContext {
                origin: request_origin,
                origin_downgrade: downgrade,
                referer: &referer,
                initiated: journey.initiated,
                site: SiteContext {
                    fetch_site,
                    initiator: journey.initiator.as_ref(),
                    chain: &journey.chain,
                },
            },
            &journey.url,
            &journey.method,
            &journey.body,
            journey.authorized.as_ref().or(journey.extra.as_ref()),
            strip_sensitive,
            header_order,
        )
    }

    pub(super) fn audit_copy(&self, headers: &[HeaderPair]) -> Vec<(String, String)> {
        if self.inner.audit_tls.is_none() {
            return Vec::new();
        }
        headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub(in crate::core::session) fn merge_session_headers(
        &self,
        headers: &mut Vec<HeaderPair>,
        caller_has: &dyn Fn(&str) -> bool,
        strip_sensitive: bool,
        target: &Url,
    ) {
        for (k, v) in self.session_defaults(target) {
            if caller_has(k) || (strip_sensitive && cross_origin_stripped(k)) {
                continue;
            }
            match headers
                .iter()
                .position(|(hk, _)| hk.eq_ignore_ascii_case(k))
            {
                Some(pos) => headers[pos].1 = Cow::Owned(v.to_owned()),
                None => headers.push((Cow::Owned(k.to_owned()), Cow::Owned(v.to_owned()))),
            }
        }
    }

    fn session_defaults<'a>(&'a self, target: &Url) -> impl Iterator<Item = (&'a str, &'a str)> {
        let bearer = self
            .inner
            .bearer
            .as_ref()
            .filter(|_| self.bearer_in_scope(target));
        let defaults = &self.inner.default_headers;
        let slot = bearer.map_or(defaults.len(), |b| b.slot.min(defaults.len()));
        let pair = |(k, v): &'a (String, String)| (k.as_str(), v.as_str());
        self.inner
            .header_style
            .append()
            .iter()
            .map(pair)
            .chain(defaults[..slot].iter().map(pair))
            .chain(bearer.map(|b| (AUTHORIZATION.as_str(), b.value.as_str())))
            .chain(defaults[slot..].iter().map(pair))
    }

    fn bearer_in_scope(&self, target: &Url) -> bool {
        self.inner
            .base_url
            .as_ref()
            .is_none_or(|base| base.origin() == target.origin())
    }

    pub(in crate::core::session) fn add_jar_cookie(
        &self,
        headers: &mut Vec<HeaderPair>,
        url: &Url,
        site: &SiteContext<'_>,
        method: &str,
    ) {
        if headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        {
            return;
        }
        let chain_rule = self.samesite_checks_redirect_chain();
        let computed = || {
            if chain_rule {
                site.fetch_site == FetchSite::CrossSite
            } else {
                site.initiator
                    .is_none_or(|initiator| FetchSite::of(initiator, url) == FetchSite::CrossSite)
            }
        };
        let cross_site = match header_value(headers, "sec-fetch-site") {
            Some(sent) if sent.eq_ignore_ascii_case(site.fetch_site.as_str()) => computed(),
            Some(sent) if sent.eq_ignore_ascii_case("none") => {
                chain_rule
                    && site.chain.first().is_some_and(|first| {
                        FetchSite::across(first, site.chain.iter().chain([url]))
                            == FetchSite::CrossSite
                    })
            }
            Some(sent) => sent.eq_ignore_ascii_case(FetchSite::CrossSite.as_str()),
            None => computed(),
        };
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| method.eq_ignore_ascii_case(m));
        let top_level = header_value(headers, "sec-fetch-dest")
            .is_none_or(|dest| dest.eq_ignore_ascii_case("document"));
        if let Some(cookie_val) =
            self.inner
                .cookie_jar
                .cookie_header_for(url, cross_site, safe_method && top_level)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }
    }

    fn samesite_checks_redirect_chain(&self) -> bool {
        self.inner
            .identity
            .map(super::super::Identity::http)
            .or(self.inner.browser)
            .is_some_and(|browser| browser.family().samesite_checks_redirect_chain())
    }

    pub(in crate::core::session) fn session_header_order(&self) -> Option<Cow<'_, [String]>> {
        self.inner
            .header_order
            .as_deref()
            .or_else(|| self.inner.header_style.order())
            .map(Cow::Borrowed)
    }
}

const REQUEST_BODY_HEADERS: [&str; 4] = [
    "content-encoding",
    "content-language",
    "content-location",
    "content-type",
];

const CORS_MODES: [&str; 2] = ["cors", "websocket"];

fn advertise_codecs(headers: &mut Vec<HeaderPair>, codecs: Option<String>) {
    match codecs {
        Some(list) => {
            for (name, value) in headers.iter_mut() {
                if name.eq_ignore_ascii_case("accept-encoding") {
                    *value = Cow::Owned(list.clone());
                }
            }
        }
        None => headers.retain(|(name, _)| !name.eq_ignore_ascii_case("accept-encoding")),
    }
}

fn add_referer(headers: &mut Vec<HeaderPair>, referer: &str) {
    if !referer.is_empty() && header_value(headers, "referer").is_none() {
        headers.push(("referer".into(), Cow::Owned(referer.to_owned())));
    }
}

fn set_content_length(headers: &mut Vec<HeaderPair>, body: &Body) {
    if let Some(len) = body.len_hint()
        && (!matches!(body.0, BodyKind::Empty) || len > 0)
    {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
        headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
    }
}

fn cross_origin_stripped(name: &str) -> bool {
    sensitive_header(name) || name.eq_ignore_ascii_case(HOST.as_str())
}

fn is_request_body_header(name: &str) -> bool {
    REQUEST_BODY_HEADERS
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
}

fn header_value<'a>(headers: &'a [HeaderPair], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_ref())
}

fn shape_origin(headers: &mut Vec<HeaderPair>, method: &str, downgrade: bool) {
    if !sends_origin(method, headers) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("origin"));
    } else if downgrade && !cors_mode(headers) {
        for (k, v) in headers.iter_mut() {
            if k.eq_ignore_ascii_case("origin") {
                *v = Cow::Borrowed("null");
            }
        }
    }
}

fn cors_mode(headers: &[HeaderPair]) -> bool {
    header_value(headers, "sec-fetch-mode")
        .is_some_and(|mode| CORS_MODES.iter().any(|m| mode.eq_ignore_ascii_case(m)))
}

fn sends_origin(method: &str, headers: &[HeaderPair]) -> bool {
    if !["GET", "HEAD"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
    {
        return true;
    }
    let cors = cors_mode(headers);
    let cross_origin = header_value(headers, "sec-fetch-site")
        .is_some_and(|site| !site.eq_ignore_ascii_case(crate::FetchSite::SameOrigin.as_str()));
    cors && cross_origin
}
