use super::Draft;
use crate::core::config::Origin;
use crate::core::proxy_pool::{Leased, ProxyPool};
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
        let leased = self.acquire(&mut this)?;
        let used = self.used_proxy(&mut this)?;
        crate::trace::note_proxy(used.as_deref());
        let session = leased.as_ref().map_or(self.session, |l| &l.session);
        crate::trace::note_browser(|| session.browser());
        let mut result = session.attempt(this).await;
        match result.as_mut() {
            Ok(response) => {
                if response.proxy().is_none() {
                    response.set_proxy(used.as_deref());
                }
                self.observe(response);
            }
            Err(error) => {
                if error.proxy().is_none() {
                    error.set_proxy(used.as_deref());
                }
            }
        }
        if let Some(leased) = leased {
            if used.is_some() {
                let answered = result.as_ref().ok().and_then(|r| Origin::of(r.url()));
                leased
                    .lease
                    .record(&result, leased.origin.as_ref(), answered.as_ref());
            }
            self.last = Some(leased.lease.index());
        }
        result
    }

    fn acquire(&self, this: &mut Draft) -> Result<Option<Leased>> {
        let Some((pool, url)) = self.pool.zip(this.url()) else {
            return Ok(None);
        };
        let Some(leased) = pool.acquire_session(self.session, url, self.last)? else {
            return Ok(None);
        };
        this.carry.overrides_mut().proxy = Some(leased.lease.config().clone());
        Ok(Some(leased))
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
