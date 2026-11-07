use super::Draft;
use crate::core::config::{Origin, Proxy};
use crate::core::proxy_pool::{Lease, ProxyPool};
use crate::core::response::Response;
use crate::core::retry::Retry;
use crate::core::session::Session;
use crate::error::Result;

pub(super) struct Route<'a> {
    session: &'a Session,
    url: std::sync::Arc<url::Url>,
    origin: Option<Origin>,
    pool: Option<&'a ProxyPool>,
    policy: &'a Retry,
    last: Option<usize>,
}

impl<'a> Route<'a> {
    pub(super) fn new(
        session: &'a Session,
        url: std::sync::Arc<url::Url>,
        attempt: &Draft,
        policy: &'a Retry,
    ) -> Self {
        let pool = session
            .proxy_pool()
            .filter(|_| attempt.carry.proxy.is_none());
        let origin = pool.and_then(|_| Origin::of(&url));
        Self {
            session,
            url,
            origin,
            pool,
            policy,
            last: None,
        }
    }

    pub(super) async fn send(&mut self, mut this: Box<Draft>) -> Result<Response> {
        let lease = self.lease(&mut this);
        let used = self.used_proxy(this.carry.proxy.as_ref());
        crate::trace::note_proxy(used.as_deref());
        let leased = match &lease {
            Some(lease) => Some(lease.session(self.session)?),
            None => None,
        };
        let session = leased.as_ref().unwrap_or(self.session);
        crate::trace::note_browser(|| session.browser());
        let mut result = session.attempt(this).await;
        match result.as_mut() {
            Ok(response) => {
                response.set_proxy(used.as_deref());
                self.observe(response);
            }
            Err(error) => error.set_proxy(used.as_deref()),
        }
        if let Some(lease) = lease {
            let answered = result.as_ref().ok().and_then(|r| Origin::of(r.url()));
            lease.record(&result, self.origin.as_ref(), answered.as_ref());
            self.last = Some(lease.index());
        }
        result
    }

    fn lease(&self, this: &mut Draft) -> Option<Box<Lease>> {
        let lease = Box::new(self.pool?.lease(self.origin.as_ref(), self.last)?);
        this.carry.proxy = Some(lease.config().clone());
        Some(lease)
    }

    fn observe(&self, response: &Response) {
        self.session
            .observe_host(response.url(), response.status(), || {
                self.policy.server_wait(response)
            });
    }

    fn used_proxy(&self, proxy: Option<&Proxy>) -> Option<String> {
        let used = match proxy {
            Some(proxy) => proxy.proxy_for(&self.url).ok().flatten(),
            None => self.session.proxy_for(&self.url, None).ok().flatten(),
        };
        used.map(|proxy| proxy.as_str().to_owned())
    }
}
