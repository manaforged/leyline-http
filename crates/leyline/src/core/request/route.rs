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
    last_method: Option<http::Method>,
}

impl<'a> Route<'a> {
    pub(super) fn new(session: &'a Session, attempt: &Draft, policy: &'a Retry) -> Self {
        let pool = session
            .proxy_pool()
            .filter(|_| attempt.carry.proxy().is_none());
        Self {
            session,
            pool,
            policy,
            last: None,
            last_method: None,
        }
    }

    pub(super) fn last_method(&self) -> Option<&http::Method> {
        self.last_method.as_ref()
    }

    pub(super) async fn send(&mut self, mut this: Box<Draft>) -> Result<Response> {
        if self.session.is_shut_down() {
            return Err(crate::error::Error::shut_down());
        }
        if let Some(hook) = self.session.before_send() {
            this.hooked(self.session, hook)?;
        }
        self.last_method = Some(this.method.clone());
        let origin = self.pool.and(this.url()).and_then(Origin::of);
        let lease = self.lease(&mut this, origin.as_ref());
        let used = self.used_proxy(&mut this)?;
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
        let pool = self.pool?;
        if let Some(host) = this.url().and_then(url::Url::host_str)
            && pool.bypasses_all(host)
        {
            return None;
        }
        let lease = Box::new(pool.lease(origin, self.last)?);
        this.carry.overrides_mut().proxy = Some(lease.config().clone());
        Some(lease)
    }

    fn observe(&self, response: &Response) {
        self.session
            .observe_host(response.url(), response.status(), || {
                self.policy.server_wait(response)
            });
    }

    fn used_proxy(&self, this: &mut Draft) -> Result<Option<String>> {
        let Some(url) = this.url.as_ref() else {
            return Ok(None);
        };
        let upgraded = self.session.hsts_upgrade(url);
        let carry = &mut this.carry;
        let used = self.session.proxy_for(&upgraded, carry.proxy())?;
        let name = used.as_deref().map(|proxy| proxy.as_str().to_owned());
        if let Some(std::borrow::Cow::Owned(proxy)) = used {
            carry.overrides_mut().first_hop = Some(Some(proxy));
        } else if name.is_none() && self.session.proxy_is_custom(carry.proxy()) {
            carry.overrides_mut().first_hop = Some(None);
        }
        Ok(name)
    }
}
