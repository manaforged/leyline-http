use std::sync::Arc;

use http::{HeaderName, HeaderValue};
use url::Url;

use super::{referer_for, url_origin};
use crate::core::body::Body;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::ResponseTiming;
use crate::core::{RedirectAction, RedirectAttempt, RedirectPolicy};
use crate::util::without_userinfo;

pub(super) struct Journey {
    pub(super) original_origin: String,
    pub(super) referrer: String,
    pub(super) initiator: Option<Url>,
    pub(super) url: Arc<Url>,
    pub(super) method: String,
    pub(super) body: Body,
    pub(super) extra: Option<HeaderList>,
    pub(super) authorized: Option<HeaderList>,
    pub(super) chain: Vec<Url>,
    pub(super) tainted: bool,
    pub(super) timing: ResponseTiming,
    caller_referrer: Option<String>,
}

pub(super) fn redirect_location(
    code: u16,
    headers: &[(HeaderName, HeaderValue)],
) -> Option<String> {
    if !matches!(code, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    headers
        .iter()
        .find(|(k, _)| *k == "location")
        .map(|(_, v)| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

fn rewrites_to_get(code: u16, method: &str) -> bool {
    match code {
        301 | 302 => method.eq_ignore_ascii_case("POST"),
        303 => !method.eq_ignore_ascii_case("HEAD"),
        _ => false,
    }
}

impl Journey {
    pub(super) fn begin(
        url: Arc<Url>,
        method: String,
        body: Body,
        extra: Option<HeaderList>,
        session_referer: Option<&str>,
    ) -> Self {
        let original_origin = url_origin(&url);
        let caller_referrer = extra
            .as_ref()
            .and_then(|h| h.get("referer"))
            .and_then(|v| v.to_str().ok())
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        let referrer = caller_referrer
            .clone()
            .unwrap_or_else(|| format!("{original_origin}/"));
        let initiator = Url::parse(
            caller_referrer
                .as_deref()
                .or(session_referer)
                .unwrap_or(&referrer),
        )
        .ok();
        Self {
            original_origin,
            referrer,
            initiator,
            url,
            method,
            body,
            extra,
            authorized: None,
            chain: Vec::new(),
            tainted: false,
            timing: ResponseTiming::accumulator(),
            caller_referrer,
        }
    }

    pub(super) fn approved_hop(
        &self,
        policy: &RedirectPolicy,
        status: u16,
        location: &str,
    ) -> Option<Url> {
        let hop = without_userinfo(Url::clone(&self.url));
        let action = policy.action(RedirectAttempt {
            status,
            url: &hop,
            location: Some(location),
            previous: &self.chain,
        });
        (action != RedirectAction::Stop).then_some(hop)
    }

    pub(super) fn follow(
        &mut self,
        hop: Url,
        status: u16,
        location: &str,
        replay: Option<Body>,
    ) -> Result<()> {
        self.chain.push(hop);
        let from = url_origin(&self.url);
        self.url = Arc::new(self.url.join(location).map_err(Error::from_url_parse)?);
        let to = url_origin(&self.url);
        self.tainted |= from != to && self.original_origin != to;
        self.retarget_referer(&to)?;
        if !matches!(self.url.scheme(), "http" | "https") {
            return Err(Error::new(Kind::Redirect).with_message(format!(
                "refusing to follow redirect to non-http(s) scheme `{}`",
                self.url.scheme()
            )));
        }
        self.carry_body(status, replay)
    }

    fn retarget_referer(&mut self, to_origin: &str) -> Result<()> {
        if let (Some(caller), Some(extra)) = (self.caller_referrer.as_deref(), self.extra.as_mut())
        {
            extra.remove_all("referer");
            let hop_referer = referer_for(Some(caller), to_origin);
            if !hop_referer.is_empty() {
                extra.set("referer", hop_referer)?;
            }
        }
        Ok(())
    }

    fn carry_body(&mut self, status: u16, replay: Option<Body>) -> Result<()> {
        if rewrites_to_get(status, &self.method) {
            self.method = "GET".to_string();
            self.body = Body::default();
            if let Some(extra) = self.extra.as_mut() {
                extra.remove_where(|name| {
                    let name = name.as_str();
                    name.starts_with("content-") || name == "transfer-encoding"
                });
            }
        } else if let Some(replay) = replay {
            self.body = replay;
        } else {
            return Err(Error::new(Kind::Redirect).with_message(format!(
                "cannot follow {status} redirect: streaming request bodies are \
                 not replayable. Buffer the body before sending, or set \
                 RedirectPolicy::none()."
            )));
        }
        Ok(())
    }
}
