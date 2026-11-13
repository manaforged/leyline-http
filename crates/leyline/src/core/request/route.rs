use super::Draft;
use crate::core::config::Origin;
use crate::core::proxy_pool::{Lease, ProxyPool};
use crate::core::response::Response;
use crate::core::retry::Retry;
use crate::core::session::Session;
use crate::error::Result;

pub(super) struct Route<'a> {
    session: &'a Session,
    pool: Option<&'a ProxyPool>,
    policy: &'a Retry,
    last: Option<usize>,
}

impl<'a> Route<'a> {
    pub(super) fn new(session: &'a Session, attempt: &Draft, policy: &'a Retry) -> Self {
        let pool = session
            .proxy_pool()
            .filter(|_| attempt.carry.proxy.is_none());
        Self {
            session,
            pool,
            policy,
            last: None,
        }
    }

    pub(super) async fn send(&mut self, mut this: Box<Draft>) -> Result<Response> {
        if let Some(hook) = self.session.before_send() {
            this.hooked(hook)?;
        }
        let origin = self.pool.and(this.url()).and_then(Origin::of);
        let lease = self.lease(&mut this, origin.as_ref());
        let used = self.used_proxy(&mut this);
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
            lease.record(&result, origin.as_ref(), answered.as_ref());
            self.last = Some(lease.index());
        }
        result
    }

    fn lease(&self, this: &mut Draft, origin: Option<&Origin>) -> Option<Box<Lease>> {
        let lease = Box::new(self.pool?.lease(origin, self.last)?);
        this.carry.proxy = Some(lease.config().clone());
        Some(lease)
    }

    fn observe(&self, response: &Response) {
        self.session
            .observe_host(response.url(), response.status(), || {
                self.policy.server_wait(response)
            });
    }

    fn used_proxy(&self, this: &mut Draft) -> Option<String> {
        let carry = &mut this.carry;
        let chosen = self
            .session
            .proxy_for(this.url.as_deref()?, carry.proxy.as_ref());
        let used = chosen.ok()?;
        let name = used.as_deref().map(|proxy| proxy.as_str().to_owned());
        if let Some(std::borrow::Cow::Owned(proxy)) = used {
            carry.first_hop = Some(Some(proxy));
        } else if name.is_none() && self.session.proxy_is_custom(carry.proxy.as_ref()) {
            carry.first_hop = Some(None);
        }
        name
    }
}
