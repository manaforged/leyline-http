use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures_util::Stream;
use http::Method;
use url::Url;

use crate::core::config::{BodyRules, Origin, Proxy, Redirect, Timeout};
use crate::core::digest::DigestAuth;
use crate::core::error::Result;
use crate::core::headers::HeaderList;
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::retry::Retry;
use crate::core::session::Session;
use crate::profile::RequestKind;
use crate::util::sensitive_header;

const NEXT: &str = "next";

type PageFuture = Pin<Box<dyn Future<Output = Result<Response>> + Send>>;

pub(crate) struct Pages {
    template: Template,
    pending: Option<PageFuture>,
    visited: HashSet<Url>,
}

struct Template {
    session: Session,
    first: Option<Url>,
    preset: Option<RequestKind>,
    preset_user: bool,
    headers: HeaderList,
    timeouts: Option<Timeout>,
    stream_response: bool,
    retry_policy: Retry,
    digest_auth: Option<DigestAuth>,
    proxy: Option<Proxy>,
    header_order: Option<Vec<String>>,
    redirect: Option<Redirect>,
    initiator: Option<Url>,
    label: Option<String>,
    status_errors: bool,
    body_rules: BodyRules,
}

impl RequestBuilder {
    /// Sends the request and follows each `Link: rel="next"` header, yielding one response per page.
    pub fn pages(self) -> impl Stream<Item = Result<Response>> + Send + Unpin + use<> {
        let template = Template::of(&self);
        let visited = template.first.iter().cloned().collect();
        Pages {
            template,
            pending: Some(Box::pin(self.send())),
            visited,
        }
    }
}

impl Template {
    fn of(first: &RequestBuilder) -> Self {
        Self {
            session: first.session.clone(),
            first: Url::parse(&first.url).ok(),
            preset: first.preset,
            preset_user: first.preset_user,
            headers: first.headers.clone(),
            timeouts: first.timeouts,
            stream_response: first.stream_response,
            retry_policy: first.retry_policy.clone(),
            digest_auth: first.digest_auth.clone(),
            proxy: first.proxy.clone(),
            header_order: first.header_order.clone(),
            redirect: first.redirect.clone(),
            initiator: first.initiator.clone(),
            label: first.label.clone(),
            status_errors: first.status_errors,
            body_rules: first.body_rules,
        }
    }

    fn downgrades(&self, url: &Url) -> bool {
        self.first
            .as_ref()
            .is_some_and(|first| first.scheme() == "https" && url.scheme() == "http")
    }

    fn same_origin(&self, url: &Url) -> bool {
        let first = self.first.as_ref().and_then(Origin::of);
        first.is_some() && first == Origin::of(url)
    }

    fn follow(&self, url: &Url) -> RequestBuilder {
        let mut next = RequestBuilder::new(&self.session, Method::GET, url.as_str());
        if self.preset_user {
            next.preset = self.preset;
            next.preset_user = true;
        }
        next.headers = self.headers.clone();
        next.timeouts = self.timeouts;
        next.stream_response = self.stream_response;
        next.retry_policy = self.retry_policy.clone();
        if self.same_origin(url) {
            next.digest_auth = self.digest_auth.clone();
        } else {
            next.headers
                .remove_where(|name| sensitive_header(name.as_str()));
        }
        next.proxy = self.proxy.clone();
        next.header_order = self.header_order.clone();
        next.redirect = self.redirect.clone();
        next.initiator = self.initiator.clone();
        next.label = self.label.clone();
        next.status_errors = self.status_errors;
        next.body_rules = self.body_rules;
        next.trusted_origin = self.first.clone();
        next
    }
}

impl Pages {
    fn queue_next(&mut self, response: &Response) {
        self.visited.insert(response.url().clone());
        let Some(url) = response.link(NEXT) else {
            return;
        };
        if self.template.downgrades(&url) {
            return;
        }
        if self.visited.insert(url.clone()) {
            self.pending = Some(Box::pin(self.template.follow(&url).send()));
        }
    }
}

impl Stream for Pages {
    type Item = Result<Response>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let Some(pending) = this.pending.as_mut() else {
            return Poll::Ready(None);
        };
        let result = ready!(pending.as_mut().poll(cx));
        this.pending = None;
        if let Ok(response) = &result {
            this.queue_next(response);
        }
        Poll::Ready(Some(result))
    }
}

impl std::fmt::Debug for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pages")
            .field("visited", &self.visited.len())
            .field("done", &self.pending.is_none())
            .finish()
    }
}
