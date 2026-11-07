use std::sync::Arc;

use url::Url;

use super::super::helpers::header_values;
use super::{referer_for, url_origin};
use crate::core::Redirect;
use crate::core::attempt::Attempt;
use crate::core::body::Body;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::ResponseTiming;
use crate::util::without_userinfo;

pub(super) struct Journey {
    original: Arc<Url>,
    original_origin_len: Option<usize>,
    referrer: Option<String>,
    pub(super) initiator: Option<Arc<Url>>,
    pub(super) initiated: bool,
    pub(super) url: Arc<Url>,
    pub(super) method: http::Method,
    pub(super) body: Body,
    pub(super) extra: Option<HeaderList>,
    pub(super) authorized: Option<HeaderList>,
    pub(super) chain: Vec<Url>,
    pub(super) tainted: bool,
    pub(super) timing: ResponseTiming,
    caller_referrer: Option<String>,
    trusted_origin: Option<String>,
}

pub(super) fn redirect_location(
    code: u16,
    headers: &crate::core::response_head::Fields,
) -> Option<String> {
    if !matches!(code, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    header_values(headers.map(), http::header::LOCATION)
        .next()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
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
        method: http::Method,
        body: Body,
        extra: Option<HeaderList>,
        page: Option<Url>,
        session_referer: Option<&str>,
    ) -> Self {
        let original_origin = url_origin(&url);
        let caller_referrer = extra
            .as_ref()
            .and_then(|h| h.get("referer"))
            .and_then(|v| v.to_str().ok())
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        let caller_sets_referer = extra.as_ref().is_some_and(|h| h.get("referer").is_some());
        let referred = if caller_sets_referer {
            caller_referrer.as_deref()
        } else {
            session_referer
        }
        .and_then(|r| url.join(r).ok());
        let initiated = page.is_some() || referred.is_some();
        let referrer = page
            .as_ref()
            .map(|page| page.as_str().to_owned())
            .or_else(|| caller_referrer.clone());
        let initiator = page
            .or(referred)
            .map(Arc::new)
            .or_else(|| origin_root(&url, &original_origin));
        let original_origin_len = match &original_origin {
            std::borrow::Cow::Borrowed(origin) => Some(origin.len()),
            std::borrow::Cow::Owned(_) => None,
        };
        Self {
            original: Arc::clone(&url),
            original_origin_len,
            referrer,
            initiator,
            initiated,
            url,
            method,
            body,
            extra,
            authorized: None,
            chain: Vec::new(),
            tainted: false,
            timing: ResponseTiming::accumulator(),
            caller_referrer,
            trusted_origin: None,
        }
    }

    pub(super) fn referrer(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        if let Some(referrer) = &self.referrer {
            return Cow::Borrowed(referrer);
        }
        match self.original_origin() {
            Cow::Borrowed(origin) if self.original.path().starts_with('/') => {
                Cow::Borrowed(&self.original.as_str()[..=origin.len()])
            }
            origin => Cow::Owned(format!("{origin}/")),
        }
    }

    pub(super) fn original_origin(&self) -> std::borrow::Cow<'_, str> {
        match self.original_origin_len {
            Some(len) => std::borrow::Cow::Borrowed(&self.original.as_str()[..len]),
            None => url_origin(&self.original),
        }
    }

    pub(super) fn origin_of<'a>(&'a self, url: &'a Url) -> std::borrow::Cow<'a, str> {
        if std::ptr::eq(url, Arc::as_ptr(&self.original)) {
            return self.original_origin();
        }
        url_origin(url)
    }

    pub(super) fn trusting(mut self, trusted: Option<&Url>) -> Self {
        self.trusted_origin = trusted.map(|url| url_origin(url).into_owned());
        self
    }

    pub(super) fn strips_credentials(&self, origin: &str) -> bool {
        match &self.trusted_origin {
            Some(trusted) => origin != trusted,
            None => !self.chain.is_empty() && origin != self.original_origin(),
        }
    }

    pub(super) fn approved_hop(
        &self,
        policy: &Redirect,
        status: u16,
        location: &str,
    ) -> Option<Url> {
        let hop = without_userinfo(Url::clone(&self.url));
        let follows = policy.follows(&Attempt::redirect(
            status,
            &hop,
            Some(location),
            &self.chain,
        ));
        follows.then_some(hop)
    }

    pub(super) fn follow(
        &mut self,
        hop: Url,
        status: u16,
        location: &str,
        replay: Option<Body>,
    ) -> Result<()> {
        self.chain.push(hop);
        let from = url_origin(&self.url).into_owned();
        self.url = Arc::new(self.url.join(location).map_err(Error::from)?);
        let to = url_origin(&self.url).into_owned();
        self.tainted |= from != to && self.original_origin() != to;
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
                extra.set("referer", hop_referer.into_owned())?;
            }
        }
        Ok(())
    }

    fn carry_body(&mut self, status: u16, replay: Option<Body>) -> Result<()> {
        if rewrites_to_get(status, self.method.as_str()) {
            self.method = http::Method::GET;
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
                 Redirect::none()."
            )));
        }
        Ok(())
    }
}

fn origin_root(url: &Arc<Url>, origin: &str) -> Option<Arc<Url>> {
    if matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return Some(Arc::clone(url));
    }
    Url::parse(&format!("{origin}/")).ok().map(Arc::new)
}
