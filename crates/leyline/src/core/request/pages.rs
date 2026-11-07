use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use futures_util::Stream;
use http::Method;
use url::Url;

use super::{Carry, RequestBuilder};
use crate::core::config::Origin;
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::session::Session;
use crate::error::Result;
use crate::util::cross_origin_header;

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
    headers: HeaderList,
    carry: Carry,
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
            first: first.draft.url().cloned(),
            headers: first.draft.headers.clone(),
            carry: first.draft.carry.clone(),
        }
    }

    fn downgrades(&self, url: &Url) -> bool {
        self.first
            .as_ref()
            .is_some_and(|first| Origin::downgrades(first, url))
    }

    fn follow(&self, url: &Url) -> RequestBuilder {
        let mut next = RequestBuilder::new(
            &self.session,
            Method::GET,
            Some(std::sync::Arc::new(url.clone())),
        );
        let preset = next.draft.carry.preset;
        next.draft.carry = self.carry.clone();
        if !self.carry.preset_user {
            next.draft.carry.preset = preset;
        }
        next.draft.headers = self.headers.clone();
        if !self
            .first
            .as_ref()
            .is_some_and(|first| Origin::same(first, url))
        {
            next.draft.carry.digest_auth = None;
            next.draft
                .headers
                .remove_where(|name| cross_origin_header(name.as_str()));
        }
        next.trusting(self.first.clone())
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
