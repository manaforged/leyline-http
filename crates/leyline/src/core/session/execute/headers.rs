use std::borrow::Cow;

use http::Method;
use http::header::AUTHORIZATION;
use url::Url;

use super::super::header_merge::apply_extra_headers;
use super::journey::Journey;
use super::{RequestContext, SiteContext, fetch_site_for, referer_for};
use crate::core::FetchSite;
use crate::core::config::Origin;
use crate::core::headers::{HeaderList, reorder};
use crate::core::request_headers::{self, HeaderKey};
use crate::core::session::Session;
use crate::header_str::HeaderStr;
use crate::profile::RequestKind;
use crate::profile::preset::{HeaderContext, Plan};
use crate::util::{cross_origin_header, header_first};
use crate::wire::HeaderPair;
use crate::wire::RequestHeaders;
use crate::wire::{Body, BodyKind, Scheme};

impl Session {
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is a distinct per-attempt input with no shared owner"
    )]
    pub(super) fn attempt_headers(
        &self,
        plan: &Plan,
        request: RequestContext<'_>,
        current_url: &Url,
        current_method: &Method,
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
        header_order: Option<&[String]>,
    ) -> Vec<HeaderPair> {
        let referer = request.referer;
        let ctx = self.header_context(&request);
        let mut headers = plan.headers(&ctx, !matches!(current_body.0, BodyKind::Empty));
        if plan.has_origin() {
            shape_origin(&mut headers, current_method, request.origin_downgrade);
        }
        let caller_has = |name: &str| {
            extra_headers
                .is_some_and(|h| h.iter().any(|(k, _)| k.as_str().eq_ignore_ascii_case(name)))
        };
        self.merge_session_headers(&mut headers, &caller_has, strip_sensitive, current_url);

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive);
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

    fn header_context<'a>(&'a self, request: &RequestContext<'a>) -> HeaderContext<'a> {
        let referer = request.referer;
        HeaderContext {
            origin: request.origin,
            referer,
            fetch_site: request.site.fetch_site.as_str(),
            navigation_site: if request.initiated {
                request.site.fetch_site.as_str()
            } else {
                FetchSite::None.as_str()
            },
            navigation_referer: if request.initiated { referer } else { "" },
            ..HeaderContext::session(
                &self.inner.user_agent,
                &self.inner.sec_ch_ua,
                &self.inner.accept_language,
                self.inner.platform,
            )
        }
    }

    pub(super) fn leg_headers(
        &self,
        journey: &Journey,
        preset: Option<RequestKind>,
        header_order: Option<&[String]>,
    ) -> RequestHeaders {
        let plan = self.inner.header_plan.plan(preset);
        let origin = journey.origin_of(&journey.url);
        let referrer = (journey.initiated || plan.reads_referer()).then(|| journey.referrer());
        let referer = referrer.as_deref().map_or(Cow::Borrowed(""), |referrer| {
            referer_for(Some(referrer), &journey.url, &origin)
        });
        let fetch_site = if plan.reads_site() || !self.inner.inputs.jar().is_empty() {
            fetch_site_for(journey.initiator.as_deref(), &journey.chain, &journey.url)
        } else {
            FetchSite::CrossSite
        };
        let strip_sensitive = journey.strips_credentials();
        let initiator = journey
            .initiator
            .as_deref()
            .filter(|url| Scheme::of(url).is_some());
        let initiator_origin = initiator.map(|url| journey.origin_of(url));
        let downgrade = initiator.is_some_and(|url| Origin::downgrades(url, &journey.url));
        let original_origin = journey.original_origin();
        let request_origin = match (&initiator_origin, journey.tainted) {
            (_, true) => "null",
            (Some(origin), false) => origin.as_ref(),
            (None, false) => original_origin.as_ref(),
        };
        let extra = journey.authorized.as_ref().or(journey.extra.as_ref());
        let build = || {
            self.attempt_headers(
                plan,
                RequestContext {
                    origin: request_origin,
                    origin_downgrade: downgrade,
                    referer: &referer,
                    initiated: journey.initiated,
                    site: SiteContext {
                        fetch_site,
                        initiator: journey.initiator.as_deref(),
                        chain: &journey.chain,
                    },
                },
                &journey.url,
                &journey.method,
                &journey.body,
                extra,
                strip_sensitive,
                header_order,
            )
        };
        let plain = extra.is_none() && header_order.is_none();
        if !plain || strip_sensitive || !self.caches_headers(journey) {
            return build().into();
        }
        let key = HeaderKey {
            session: self.inner.header_id.get(),
            preset,
            method: journey.method.as_str(),
            origin: request_origin,
            referer: &referer,
            site: fetch_site,
            downgrade,
        };
        request_headers::cached(&key, build)
    }

    fn caches_headers(&self, journey: &Journey) -> bool {
        !journey.initiated
            && matches!(journey.body.0, BodyKind::Empty)
            && self.inner.inputs.bearer.is_none()
            && self.inner.inputs.jar().is_empty()
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
            if caller_has(k) || (strip_sensitive && cross_origin_header(k)) {
                continue;
            }
            match headers
                .iter()
                .position(|(hk, _)| hk == k || hk.eq_ignore_ascii_case(k))
            {
                Some(pos) => headers[pos].1 = v,
                None => headers.push((Cow::Owned(k.to_owned()), v)),
            }
        }
    }

    fn session_defaults<'a>(&'a self, target: &Url) -> impl Iterator<Item = (&'a str, HeaderStr)> {
        let bearer = self
            .inner
            .inputs
            .bearer
            .as_ref()
            .filter(|_| self.bearer_in_scope(target));
        let defaults = &self.inner.default_headers;
        let slot = bearer.map_or(defaults.len(), |b| b.slot.min(defaults.len()));
        let pair = |(k, v): &'a (String, HeaderStr)| (k.as_str(), v.clone());
        self.inner
            .header_plan
            .append()
            .iter()
            .map(|(k, v)| (k.as_str(), HeaderStr::from_static(v.as_str())))
            .chain(defaults[..slot].iter().map(pair))
            .chain(bearer.map(|b| (AUTHORIZATION.as_str(), b.value())))
            .chain(defaults[slot..].iter().map(pair))
    }

    pub(in crate::core::session) fn bearer_in_scope(&self, target: &Url) -> bool {
        self.inner
            .inputs
            .base_url
            .as_ref()
            .is_none_or(|base| Origin::same(base, target))
    }

    pub(in crate::core::session) fn add_jar_cookie(
        &self,
        headers: &mut Vec<HeaderPair>,
        url: &Url,
        site: &SiteContext<'_>,
        method: &Method,
    ) {
        if self.inner.inputs.jar().is_empty()
            || headers
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
        let cross_site = match header_first(headers, "sec-fetch-site").map(FetchSite::parse) {
            Some(sent) if sent == Some(site.fetch_site) => computed(),
            Some(Some(FetchSite::None)) => {
                chain_rule
                    && site.chain.first().is_some_and(|first| {
                        FetchSite::across(first, site.chain.iter().chain([url]))
                            == FetchSite::CrossSite
                    })
            }
            Some(sent) => sent == Some(FetchSite::CrossSite),
            None => computed(),
        };
        let safe_method = crate::wire::is_retrieval(method);
        let top_level = header_first(headers, "sec-fetch-dest")
            .is_none_or(|dest| dest.eq_ignore_ascii_case("document"));
        if let Some(cookie_val) =
            self.inner
                .inputs
                .jar()
                .cookie_header_for(url, cross_site, safe_method && top_level)
        {
            headers.push(("cookie".into(), HeaderStr::from(cookie_val)));
        }
    }

    fn samesite_checks_redirect_chain(&self) -> bool {
        self.inner
            .identity
            .as_ref()
            .and_then(crate::core::Identity::http)
            .or(self.inner.browser)
            .is_some_and(|browser| browser.family().samesite_checks_redirect_chain())
    }

    pub(in crate::core::session) fn session_header_order(&self) -> Option<Cow<'_, [String]>> {
        self.inner
            .header_order
            .as_deref()
            .or_else(|| self.inner.header_plan.order())
            .map(Cow::Borrowed)
    }
}

const CORS_MODES: [&str; 2] = ["cors", "websocket"];

fn add_referer(headers: &mut Vec<HeaderPair>, referer: &str) {
    if !referer.is_empty() && header_first(headers, "referer").is_none() {
        headers.push(("referer".into(), HeaderStr::from(referer)));
    }
}

fn set_content_length(headers: &mut Vec<HeaderPair>, body: &Body) {
    if let Some(len) = body.len_hint()
        && (!matches!(body.0, BodyKind::Empty) || len > 0)
    {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
        headers.insert(
            0,
            ("content-length".into(), HeaderStr::from(len.to_string())),
        );
    }
}

fn shape_origin(headers: &mut Vec<HeaderPair>, method: &Method, downgrade: bool) {
    if !sends_origin(method, headers) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("origin"));
    } else if downgrade && !cors_mode(headers) {
        for (k, v) in headers.iter_mut() {
            if k.eq_ignore_ascii_case("origin") {
                *v = HeaderStr::from_static("null");
            }
        }
    }
}

fn cors_mode(headers: &[HeaderPair]) -> bool {
    header_first(headers, "sec-fetch-mode")
        .is_some_and(|mode| CORS_MODES.iter().any(|m| mode.eq_ignore_ascii_case(m)))
}

fn sends_origin(method: &Method, headers: &[HeaderPair]) -> bool {
    if !crate::wire::is_retrieval(method) {
        return true;
    }
    let cors = cors_mode(headers);
    let cross_origin = header_first(headers, "sec-fetch-site")
        .is_some_and(|site| FetchSite::parse(site) != Some(FetchSite::SameOrigin));
    cors && cross_origin
}
