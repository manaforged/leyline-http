use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::core::headers;
use crate::error::{Error, Kind, Result};
use crate::header_str::HeaderStr;
use crate::util::coalesce::Coalesce;
use crate::util::lock;

pub(in crate::core::session) type Callback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

pub(in crate::core::session) struct Refresh {
    state: Arc<Mutex<Token>>,
    running: Coalesce<u64, std::result::Result<(), Arc<Error>>>,
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
            state: Arc::new(Mutex::new(Token {
                value: value.into(),
                version: 0,
                failure: None,
            })),
            running: Coalesce::default(),
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
        {
            let state = lock(&self.state);
            if state.version != version {
                return outcome(state.failure.as_ref());
            }
        }
        let state = Arc::clone(&self.state);
        let callback = Arc::clone(&self.callback);
        let joined = self
            .running
            .join_or_spawn(
                &version,
                move || async move {
                    {
                        let current = lock(&state);
                        if current.version != version {
                            return match &current.failure {
                                None => Ok(()),
                                Some(err) => Err(Arc::clone(err)),
                            };
                        }
                    }
                    let result = (callback)().await.and_then(|token| {
                        let value = crate::util::bearer_credentials(&token);
                        headers::value(value.as_str())?;
                        Ok(HeaderStr::from(value))
                    });
                    let mut current = lock(&state);
                    current.version = current.version.wrapping_add(1);
                    match result {
                        Ok(value) => {
                            current.value = value;
                            current.failure = None;
                            Ok(())
                        }
                        Err(error) => {
                            let error = Arc::new(error);
                            current.failure = Some(Arc::clone(&error));
                            Err(error)
                        }
                    }
                },
                |join_err| {
                    Err(Arc::new(Error::new(Kind::Request).with_message(format!(
                        "token refresh task failed: {join_err}"
                    ))))
                },
            )
            .await;
        outcome(joined.err().as_ref())
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
