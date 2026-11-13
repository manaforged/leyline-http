use std::sync::Arc;

use http::HeaderMap;

use super::download::Download;
use super::send::http_scheme;
use super::{Carry, Draft, RequestBuilder};
use crate::core::Redirect;
use crate::core::config::Proxy;
use crate::core::headers::HeaderList;
use crate::core::retry::Retry;
use crate::core::session::{BeforeSend, Protocol, Session};
use crate::deadline::Timeout;
use crate::error::{Error, Result};
use crate::profile::{HeaderPosition, RequestKind};
use crate::wire::Body;

#[derive(Clone, Default)]
struct Layout(Vec<(http::HeaderName, Option<HeaderPosition>)>);

impl Draft {
    pub(super) fn hooked(&mut self, hook: &BeforeSend) -> Result<()> {
        let layout = Layout::of(&self.headers);
        let mut request = self.as_http()?;
        let sent = request.uri().clone();
        let outcome = hook(&mut request);
        let (parts, body) = request.into_parts();
        self.body = body;
        outcome?;
        self.headers = layout.list(&parts.headers)?;
        self.method = parts.method;
        if parts.uri != sent {
            let url = absolute(&parts.uri)?;
            http_scheme(&url)?;
            self.url = Some(url);
        }
        Ok(())
    }

    pub(super) fn into_http(mut self) -> Result<http::Request<Body>> {
        let layout = Layout::of(&self.headers);
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
        if let Some(protocol) = carry.protocol.take() {
            options.insert(protocol);
        }
        if !layout.0.is_empty() {
            options.insert(layout);
        }
        if let Some(download) = self.download.take() {
            options.insert(download);
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
        let url = absolute(&parts.uri);
        let mut builder = Self::new(session, parts.method, url.as_ref().ok().cloned());
        if let Err(err) = url {
            builder.fail(err);
        }
        let options = &mut parts.extensions;
        let layout = options.remove::<Layout>().unwrap_or_default();
        match layout.list(&parts.headers) {
            Ok(headers) => builder.draft.headers = headers,
            Err(err) => builder.fail(err),
        }
        builder.draft.body = body;
        builder.draft.download = options.remove::<Box<Download>>();
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
        if let Some(protocol) = options.remove::<Protocol>() {
            builder = builder.protocol(protocol);
        }
        builder
    }
}

fn absolute(uri: &http::Uri) -> Result<Arc<url::Url>> {
    Ok(Arc::new(url::Url::parse(&uri.to_string())?))
}

impl Layout {
    fn of(headers: &HeaderList) -> Self {
        Self(
            headers
                .entries()
                .map(|entry| (entry.name.clone(), entry.anchor))
                .collect(),
        )
    }

    fn list(&self, headers: &HeaderMap) -> Result<HeaderList> {
        let mut left: Vec<_> = headers
            .keys()
            .map(|name| (name, headers.get_all(name).iter()))
            .collect();
        let mut list = HeaderList::new();
        for (name, anchor) in &self.0 {
            let values = left.iter_mut().find(|(kept, _)| *kept == name);
            let Some(value) = values.and_then(|(_, values)| values.next()) else {
                continue;
            };
            match anchor {
                Some(anchor) => list.append_anchored(*anchor, name.clone(), value.clone())?,
                None => list.append(name.clone(), value.clone())?,
            }
        }
        for (name, values) in left {
            for value in values {
                list.append(name.clone(), value.clone())?;
            }
        }
        Ok(list)
    }
}
