use std::borrow::Cow;

use http::header::HOST;
use url::Url;

use super::super::header_merge::apply_extra_headers;
use super::journey::Journey;
use super::{RequestContext, fetch_site_for, referer_for, url_origin};
use crate::core::Session;
use crate::core::body::{Body, BodyKind};
use crate::core::headers::{HeaderList, reorder};
use crate::profile::Preset;
use crate::profile::preset::HeaderPair;
use crate::util::sensitive_header;

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn attempt_headers(
        &self,
        preset: Option<Preset>,
        request: RequestContext<'_>,
        current_url: &Url,
        current_method: &str,
        redirect_chain: &[Url],
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
        header_order: Option<&[String]>,
    ) -> Vec<HeaderPair> {
        let referer = request.referer;
        let caller_referer = extra_headers
            .and_then(|h| h.get("referer"))
            .and_then(|v| v.to_str().ok())
            .filter(|r| !r.is_empty());
        let ctx = crate::profile::preset::HeaderContext {
            user_agent: &self.inner.user_agent,
            sec_ch_ua: &self.inner.sec_ch_ua,
            sec_ch_ua_mobile: self.inner.platform.mobile_flag(),
            sec_ch_ua_platform: self.inner.platform.sec_ch_platform(),
            accept_language: &self.inner.accept_language,
            origin: request.origin,
            referer,
            fetch_site: request.fetch_site.as_str(),
        };
        let mut headers = self.inner.header_style.build_headers(preset, &ctx);
        if matches!(current_body.0, BodyKind::Empty) {
            headers.retain(|(k, _)| !is_request_body_header(k));
        }
        if !sends_origin(current_method, &headers) {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("origin"));
        }
        if referer.is_empty() && caller_referer.is_none() {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("referer"));
        }
        let caller_has = |name: &str| {
            extra_headers
                .is_some_and(|h| h.iter().any(|(k, _)| k.as_str().eq_ignore_ascii_case(name)))
        };
        self.merge_session_headers(&mut headers, &caller_has, strip_sensitive);

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &cross_origin_stripped);
        }

        if let Some(len) = current_body.len_hint()
            && (!matches!(current_body.0, BodyKind::Empty) || len > 0)
        {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
            headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
        }

        self.add_jar_cookie(&mut headers, current_url, redirect_chain, current_method);

        if let Some(order) = header_order
            .map(Cow::Borrowed)
            .or_else(|| self.session_header_order())
        {
            reorder(&mut headers, &order);
        }

        headers
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
        let strip_sensitive = !journey.chain.is_empty() && origin != journey.original_origin;
        let request_origin = if journey.tainted {
            "null"
        } else {
            journey.original_origin.as_str()
        };
        self.attempt_headers(
            preset,
            RequestContext {
                origin: request_origin,
                referer: &referer,
                fetch_site,
            },
            &journey.url,
            &journey.method,
            &journey.chain,
            &journey.body,
            journey.extra.as_ref(),
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
    ) {
        for (k, v) in self
            .inner
            .header_style
            .append()
            .iter()
            .chain(self.inner.default_headers.iter())
        {
            if caller_has(k) || (strip_sensitive && cross_origin_stripped(k)) {
                continue;
            }
            match headers
                .iter()
                .position(|(hk, _)| hk.eq_ignore_ascii_case(k))
            {
                Some(pos) => headers[pos].1 = Cow::Owned(v.clone()),
                None => headers.push((Cow::Owned(k.clone()), Cow::Owned(v.clone()))),
            }
        }
    }

    pub(in crate::core::session) fn add_jar_cookie(
        &self,
        headers: &mut Vec<HeaderPair>,
        url: &Url,
        redirect_chain: &[Url],
        method: &str,
    ) {
        if headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        {
            return;
        }
        let cross_site = crate::cookie::is_cross_site(url, redirect_chain);
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| method.eq_ignore_ascii_case(m));
        if let Some(cookie_val) =
            self.inner
                .cookie_jar
                .cookie_header_for(url, cross_site, safe_method)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }
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

fn sends_origin(method: &str, headers: &[HeaderPair]) -> bool {
    if !["GET", "HEAD"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
    {
        return true;
    }
    let cors = header_value(headers, "sec-fetch-mode")
        .is_some_and(|mode| CORS_MODES.iter().any(|m| mode.eq_ignore_ascii_case(m)));
    let cross_origin = header_value(headers, "sec-fetch-site")
        .is_some_and(|site| !site.eq_ignore_ascii_case(crate::FetchSite::SameOrigin.as_str()));
    cors && cross_origin
}
