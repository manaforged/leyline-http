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
        let ctx = crate::profile::preset::HeaderContext {
            user_agent: &self.inner.user_agent,
            sec_ch_ua: &self.inner.sec_ch_ua,
            sec_ch_ua_mobile: self.inner.platform.mobile_flag(),
            sec_ch_ua_platform: self.inner.platform.sec_ch_platform(),
            accept_language: &self.inner.accept_language,
            origin,
            referer,
        };
        let mut headers = self.inner.header_style.build_headers(preset, &ctx);
        let sensitive = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower == "authorization" || lower == "proxy-authorization" || lower == "cookie"
        };

        for (k, v) in self
            .inner
            .header_style
            .append()
            .iter()
            .chain(self.inner.default_headers.iter())
        {
            let user_has_it = extra_headers
                .as_ref()
                .map(|h| h.iter().any(|(uk, _)| uk.as_str().eq_ignore_ascii_case(k)))
                .unwrap_or(false);
            if user_has_it || (strip_sensitive && sensitive(k)) {
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

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &sensitive);
        }

        if let Some(len) = current_body.len_hint()
            && (!matches!(current_body.0, BodyKind::Empty) || len > 0)
        {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
            headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
        }

        let cross_site = crate::cookie::is_cross_site(current_url, redirect_chain);
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| current_method.eq_ignore_ascii_case(m));
        let caller_cookie = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("cookie"));
        if !caller_cookie
            && let Some(cookie_val) =
                self.inner
                    .cookie_jar
                    .cookie_header_for(current_url, cross_site, safe_method)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }

        if let Some(order) = header_order
            .map(Cow::Borrowed)
            .or_else(|| self.session_header_order())
        {
            reorder(&mut headers, &order);
        }

        headers
    }
    pub(in crate::core::session) fn session_header_order(&self) -> Option<Cow<'_, [String]>> {
        self.inner
            .header_order
            .as_deref()
            .or_else(|| self.inner.header_style.order())
            .map(Cow::Borrowed)
    }
}
