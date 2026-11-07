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
            Token::Fixed(value) => HeaderStr::from(value.as_str()),
            Token::Refreshing(refresh) => refresh.value(),
        }
    }

    pub(in crate::core::session) fn version(&self) -> Option<u64> {
        match &self.value {
            Token::Fixed(_) => None,
            Token::Refreshing(refresh) => Some(refresh.version()),
        }
    }

    pub(in crate::core::session) async fn renew(&self, version: u64) -> Result<()> {
        match &self.value {
            Token::Fixed(_) => Ok(()),
            Token::Refreshing(refresh) => refresh.renew(version).await,
        }
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
    /// Sets the `Authorization: Bearer` header sent on every request.
    pub fn bearer_auth(mut self, token: &str) -> Self {
        let value = format!("Bearer {token}");
        if crate::core::headers::value(value.as_str()).is_err() {
            self.invalid_header(AUTHORIZATION);
        }
        self.remove_default(AUTHORIZATION);
        self.bearer = Some(BearerToken {
            value: Token::Fixed(value),
            slot: self.default_headers.len(),
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
