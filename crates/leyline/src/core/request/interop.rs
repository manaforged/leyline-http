use std::sync::Arc;

use http::HeaderMap;

use super::{Carry, Draft, RequestBuilder};
use crate::core::Redirect;
use crate::core::config::Proxy;
use crate::core::headers::HeaderList;
use crate::core::retry::Retry;
use crate::core::session::{BeforeSend, Session};
use crate::deadline::Timeout;
use crate::error::{Error, Result};
use crate::profile::{HeaderPosition, RequestKind};
use crate::wire::Body;

type Anchors = HeaderMap<HeaderPosition>;

impl Draft {
    pub(super) fn hooked(&mut self, hook: &BeforeSend) -> Result<()> {
        let anchors = anchors(&self.headers);
        let mut request = self.as_http()?;
        let outcome = hook(&mut request);
        let (parts, body) = request.into_parts();
        self.body = body;
        outcome?;
        self.headers = listed(&parts.headers, &anchors)?;
        Ok(())
    }

    pub(super) fn into_http(mut self) -> Result<http::Request<Body>> {
        let anchors = anchors(&self.headers);
        let mut request = self.as_http()?;
        let carry = &mut self.carry;
        let options = request.extensions_mut();
        if let Some(kind) = carry.preset.take() {
            options.insert(kind);
        }
        carry.preset_user = false;
        if let Some(timeouts) = carry.timeouts.take() {
            options.insert(timeouts);
        }
        options.insert(std::mem::take(&mut carry.retry_policy));
        if let Some(redirect) = carry.redirect.take() {
            options.insert(redirect);
        }
        if let Some(proxy) = carry.proxy.take() {
            options.insert(proxy);
        }
        if !anchors.is_empty() {
            options.insert(anchors);
        }
        options.insert(self.carry);
        Ok(request)
    }

    fn as_http(&mut self) -> Result<http::Request<Body>> {
        let url = self
            .url
            .as_deref()
            .ok_or_else(|| Error::from(url::ParseError::EmptyHost))?;
        let uri = url.as_str().parse::<http::Uri>()?;
        let mut headers = HeaderMap::new();
        for (name, value) in self.headers.iter() {
            headers.append(name.clone(), value.clone());
        }
        let mut request = http::Request::new(std::mem::take(&mut self.body));
        *request.method_mut() = self.method.clone();
        *request.uri_mut() = uri;
        *request.headers_mut() = headers;
        Ok(request)
    }
}

impl RequestBuilder {
    pub(crate) fn from_http(session: &Session, request: http::Request<Body>) -> Self {
        let (mut parts, body) = request.into_parts();
        let url = url::Url::parse(&parts.uri.to_string());
        let mut builder = Self::new(
            session,
            parts.method,
            url.as_ref().ok().cloned().map(Arc::new),
        );
        if let Err(err) = url {
            builder.fail(Error::from(err));
        }
        let options = &mut parts.extensions;
        let anchors = options.remove::<Anchors>().unwrap_or_default();
        match listed(&parts.headers, &anchors) {
            Ok(headers) => builder.draft.headers = headers,
            Err(err) => builder.fail(err),
        }
        builder.draft.body = body;
        if let Some(rest) = options.remove::<Carry>() {
            let fresh = std::mem::replace(&mut builder.draft.carry, rest);
            builder.draft.carry.preset = fresh.preset;
            builder.draft.carry.retry_policy = fresh.retry_policy;
        }
        if let Some(kind) = options.remove::<RequestKind>() {
            builder = builder.kind(kind);
        }
        if let Some(timeouts) = options.remove::<Timeout>() {
            builder = builder.timeout(timeouts);
        }
        if let Some(policy) = options.remove::<Retry>() {
            builder = builder.retry(policy);
        }
        if let Some(policy) = options.remove::<Redirect>() {
            builder = builder.redirect(policy);
        }
        if let Some(proxy) = options.remove::<Proxy>() {
            builder = builder.proxy(proxy);
        }
        builder
    }
}

fn anchors(headers: &HeaderList) -> Anchors {
    let mut anchors = Anchors::default();
    for entry in headers.entries() {
        if let Some(anchor) = entry.anchor {
            anchors.append(entry.name.clone(), anchor);
        }
    }
    anchors
}

fn listed(headers: &HeaderMap, anchors: &Anchors) -> Result<HeaderList> {
    let mut list = HeaderList::new();
    for (name, value) in headers {
        match anchors.get(name) {
            Some(anchor) => list.append_anchored(*anchor, name.clone(), value.clone())?,
            None => list.append(name.clone(), value.clone())?,
        }
    }
    Ok(list)
}
