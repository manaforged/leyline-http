use crate::core::Result;
use crate::core::config::{Origin, ProxyConfig};
use crate::core::proxy_pool::{Lease, ProxyPool};
use crate::core::response::Response;
use crate::core::retry::RetryPolicy;
use crate::core::session::Session;
use crate::core::session::execute::Attempt;

pub(super) struct Route<'a> {
    session: &'a Session,
    url: Option<url::Url>,
    origin: Option<Origin>,
    pool: Option<&'a ProxyPool>,
    policy: &'a RetryPolicy,
    last: Option<usize>,
}

impl<'a> Route<'a> {
    pub(super) fn new(session: &'a Session, attempt: &Attempt, policy: &'a RetryPolicy) -> Self {
        let url = url::Url::parse(&attempt.url).ok();
        let origin = url.as_ref().and_then(Origin::of);
        let pool = session.proxy_pool().filter(|_| attempt.proxy.is_none());
        Self {
            session,
            url,
            origin,
            pool,
            policy,
            last: None,
        }
    }

    pub(super) async fn send(
        &mut self,
        mut this: Attempt,
        rotation: Option<&ProxyConfig>,
    ) -> Result<Response> {
        let lease = self.lease(&mut this, rotation);
        let used = self.used_proxy(this.proxy.as_ref());
        crate::trace::note_proxy(used.as_deref());
        let session = match &lease {
            Some(lease) => lease.session(self.session)?,
            None => self.session.clone(),
        };
        crate::trace::note_browser(session.identity().browser());
        let mut result = session.attempt(this).await;
        let answered = match result.as_mut() {
            Ok(response) => {
                response.set_proxy(used.as_deref());
                self.observe(response);
                Origin::of(response.url())
            }
            Err(error) => {
                error.set_proxy(used.as_deref());
                None
            }
        };
        if let Some(lease) = lease {
            lease.record(&result, self.origin.as_ref(), answered.as_ref());
            self.last = Some(lease.index());
        }
        result
    }

    fn lease(&self, this: &mut Attempt, rotation: Option<&ProxyConfig>) -> Option<Lease> {
        let Some(pool) = self.pool else {
            if let Some(proxy) = rotation {
                this.proxy = Some(proxy.clone());
            }
            return None;
        };
        let lease = pool.lease(self.origin.as_ref(), self.last)?;
        this.proxy = Some(lease.config().clone());
        Some(lease)
    }

    fn observe(&self, response: &Response) {
        self.session.host_limits().observe(
            response.url(),
            response.status(),
            self.policy.server_wait(response),
        );
    }

    fn used_proxy(&self, proxy: Option<&ProxyConfig>) -> Option<String> {
        let used = match (proxy, &self.url) {
            (Some(proxy), Some(url)) => proxy.proxy_for(url).ok().flatten(),
            (Some(proxy), None) => proxy.primary(),
            (None, Some(url)) => self.session.proxy_for(url, None).ok().flatten(),
            (None, None) => None,
        };
        used.map(str::to_owned)
    }
}
