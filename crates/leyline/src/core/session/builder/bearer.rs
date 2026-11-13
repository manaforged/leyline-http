use std::future::Future;
use std::sync::Arc;

use super::{AUTHORIZATION, SessionBuilder};
use crate::error::Result;
use crate::header_str::HeaderStr;

mod refresh;
pub(in crate::core::session) use refresh::Callback;
use refresh::Refresh;

#[derive(Clone)]
enum Token {
    Fixed(String),
    Basic(String),
    Refreshing(Arc<Refresh>),
}

#[derive(Clone)]
pub(crate) struct BearerToken {
    value: Token,
    pub(crate) slot: usize,
}

impl BearerToken {
    pub(in crate::core::session) fn value(&self) -> HeaderStr {
        match &self.value {
            Token::Fixed(value) | Token::Basic(value) => HeaderStr::from(value.as_str()),
            Token::Refreshing(refresh) => refresh.value(),
        }
    }

    pub(in crate::core::session) fn version(&self) -> Option<u64> {
        match &self.value {
            Token::Fixed(_) | Token::Basic(_) => None,
            Token::Refreshing(refresh) => Some(refresh.version()),
        }
    }

    pub(in crate::core::session) async fn renew(&self, version: u64) -> Result<()> {
        match &self.value {
            Token::Fixed(_) | Token::Basic(_) => Ok(()),
            Token::Refreshing(refresh) => refresh.renew(version).await,
        }
    }

    pub(super) fn refreshable(&self) -> bool {
        !matches!(self.value, Token::Basic(_))
    }

    pub(super) fn refreshing(mut self, callback: Option<Callback>) -> Self {
        if let Some(callback) = callback {
            let same = matches!(&self.value, Token::Refreshing(refresh) if refresh.uses(&callback));
            if !same {
                let value = self.value().as_str().to_owned();
                self.value = Token::Refreshing(Arc::new(Refresh::new(value, callback)));
            }
        }
        self
    }
}

impl SessionBuilder {
    /// Sets the `Authorization: Bearer` header, sent to every origin, or only to the `base_url` origin when one is set, and never across a cross-origin redirect.
    pub fn bearer_auth(self, token: &str) -> Self {
        self.credential(Token::Fixed(format!("Bearer {token}")))
    }

    /// Sets the `Authorization` header to HTTP Basic credentials with the same origin scope as `bearer_auth`, replacing a bearer token or default `Authorization` header set before it.
    pub fn basic_auth(self, user: &str, password: &str) -> Self {
        self.credential(Token::Basic(crate::util::basic_credentials(user, password)))
    }

    fn credential(mut self, value: Token) -> Self {
        let token = BearerToken { value, slot: 0 };
        if crate::core::headers::value(token.value().as_str()).is_err() {
            self.invalid_header(AUTHORIZATION);
        }
        self.remove_default(AUTHORIZATION);
        self.bearer = Some(BearerToken {
            slot: self.default_headers.len(),
            ..token
        });
        self
    }

    /// Refreshes a scoped bearer token once after a 401, sharing refresh work across concurrent requests.
    pub fn refresh_token<F, Fut>(mut self, refresh: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static,
    {
        self.refresh_token = Some(Arc::new(move || Box::pin(refresh())));
        self
    }

    pub(super) fn remove_default(&mut self, name: &str) {
        if let Some(bearer) = self.bearer.as_mut() {
            let removed = self.default_headers[..bearer.slot]
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(name))
                .count();
            bearer.slot -= removed;
        }
        self.default_headers
            .retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    pub(super) fn push_default(&mut self, name: String, value: String) {
        match self
            .bearer
            .take_if(|_| name.eq_ignore_ascii_case(AUTHORIZATION))
        {
            Some(bearer) => self.default_headers.insert(bearer.slot, (name, value)),
            None => self.default_headers.push((name, value)),
        }
    }
}
