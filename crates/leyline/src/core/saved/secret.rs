use crate::core::ProxyUrl;
use crate::core::error::{Error, Result};

use super::SavedIn;

impl SavedIn {
    pub(super) fn resolved_proxy(&self) -> Result<Option<ProxyUrl>> {
        let (Some(proxy), Some(var)) = (&self.proxy, &self.proxy_password_env) else {
            return Ok(self.proxy.clone());
        };
        let password = std::env::var(var).map_err(|e| {
            Error::config(format!(
                "the proxy password variable {var} is not set or not valid UTF-8"
            ))
            .with_source(e)
        })?;
        proxy.with_password(&password).map(Some)
    }
}
