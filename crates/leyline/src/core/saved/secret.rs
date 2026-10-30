use url::Url;

use crate::core::ProxyUrl;
use crate::core::error::Result;

use super::{SavedIn, config};

impl SavedIn {
    pub(super) fn resolved_proxy(&self) -> Result<Option<ProxyUrl>> {
        let (Some(proxy), Some(var)) = (&self.proxy, &self.proxy_password_env) else {
            return Ok(self.proxy.clone());
        };
        let password = std::env::var(var).map_err(|e| {
            config(format!(
                "the proxy password variable {var} is not set or not valid UTF-8"
            ))
            .with_source(e)
        })?;
        let mut url = parse(proxy).ok_or_else(|| config("the saved proxy URL is invalid"))?;
        url.set_password(Some(&password))
            .map_err(|()| config("the saved proxy URL cannot carry a password"))?;
        ProxyUrl::parse(url.as_str()).map(Some)
    }
}

pub(super) fn strip_password(proxy: &ProxyUrl) -> ProxyUrl {
    let Some(mut url) = parse(proxy).filter(|url| url.password().is_some()) else {
        return proxy.clone();
    };
    if url.set_password(None).is_err() {
        return proxy.clone();
    }
    ProxyUrl::parse(url.as_str()).unwrap_or_else(|_| proxy.clone())
}

fn parse(proxy: &ProxyUrl) -> Option<Url> {
    Url::parse(&String::from(proxy.clone())).ok()
}
