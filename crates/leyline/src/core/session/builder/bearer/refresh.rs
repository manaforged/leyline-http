use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::core::headers;
use crate::core::{Error, Result};
use crate::header_str::HeaderStr;
use crate::util::lock;

pub(in crate::core::session) type Callback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

pub(in crate::core::session) struct Refresh {
    state: Mutex<Token>,
    gate: tokio::sync::Mutex<()>,
    callback: Callback,
}

struct Token {
    value: HeaderStr,
    version: u64,
    failure: Option<Arc<Error>>,
}

impl Refresh {
    pub(super) fn new(value: String, callback: Callback) -> Self {
        Self {
            state: Mutex::new(Token {
                value: value.into(),
                version: 0,
                failure: None,
            }),
            gate: tokio::sync::Mutex::new(()),
            callback,
        }
    }

    pub(super) fn uses(&self, callback: &Callback) -> bool {
        Arc::ptr_eq(&self.callback, callback)
    }

    pub(super) fn value(&self) -> HeaderStr {
        lock(&self.state).value.clone()
    }

    pub(super) fn version(&self) -> u64 {
        lock(&self.state).version
    }

    pub(super) async fn renew(&self, version: u64) -> Result<()> {
        let _gate = self.gate.lock().await;
        {
            let state = lock(&self.state);
            if state.version != version {
                return outcome(state.failure.as_ref());
            }
        }
        let result = (self.callback)().await.and_then(|token| {
            let value = format!("Bearer {token}");
            headers::value(value.as_str())?;
            Ok(HeaderStr::from(value))
        });
        let mut state = lock(&self.state);
        state.version = state.version.wrapping_add(1);
        match result {
            Ok(value) => {
                state.value = value;
                state.failure = None;
            }
            Err(error) => state.failure = Some(Arc::new(error)),
        }
        outcome(state.failure.as_ref())
    }
}

fn outcome(error: Option<&Arc<Error>>) -> Result<()> {
    match error {
        None => Ok(()),
        Some(error) => Err(Error::new(error.kind())
            .with_message("token refresh failed")
            .with_source(Arc::clone(error))),
    }
}
