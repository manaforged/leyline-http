use std::borrow::Cow;

use url::Url;

use super::super::header_merge::apply_extra_headers;
use crate::core::Session;
use crate::core::body::{Body, BodyKind};
use crate::core::headers::{HeaderList, reorder};
use crate::profile::Preset;
use crate::profile::preset::HeaderPair;

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn attempt_headers(
        &self,
        preset: Option<Preset>,
        origin: &str,
        referer: &str,
        current_url: &Url,
        current_method: &str,
        redirect_chain: &[String],
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
        header_order: Option<&[String]>,
    ) -> Vec<HeaderPair> {
        let caller_referer = extra_headers
            .and_then(|h| h.get("referer"))
            .and_then(|v| v.to_str().ok())
            .filter(|r| !r.is_empty());
        let fetch_site = caller_referer
            .map(|r| crate::fetch_site(r, current_url.as_str()))
            .unwrap_or("same-origin");
        let ctx = crate::profile::preset::HeaderContext {
            user_agent: &self.inner.user_agent,
            sec_ch_ua: &self.inner.sec_ch_ua,
            sec_ch_ua_mobile: self.inner.platform.mobile_flag(),
            sec_ch_ua_platform: self.inner.platform.sec_ch_platform(),
            accept_language: &self.inner.accept_language,
            origin,
            referer,
            fetch_site,
        };
        let mut headers = self.inner.header_style.build_headers(preset, &ctx);
        if referer.is_empty() && caller_referer.is_none() {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("referer"));
        }
        let caller_has = |name: &str| {
            extra_headers
                .is_some_and(|h| h.iter().any(|(k, _)| k.as_str().eq_ignore_ascii_case(name)))
        };
        self.merge_session_headers(&mut headers, &caller_has, strip_sensitive);

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &is_sensitive);
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
            if caller_has(k) || (strip_sensitive && is_sensitive(k)) {
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
        redirect_chain: &[String],
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

fn is_sensitive(name: &str) -> bool {
    ["authorization", "proxy-authorization", "cookie"]
        .iter()
        .any(|s| name.eq_ignore_ascii_case(s))
}
