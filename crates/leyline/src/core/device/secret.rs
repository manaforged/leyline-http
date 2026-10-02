use url::Url;

use crate::core::ProxyUrl;
use crate::core::error::Result;

use super::{Device, config};

#[derive(PartialEq, Eq)]
pub(super) struct ProxyKey {
    scheme: String,
    username: String,
    host: Option<String>,
    port: Option<u16>,
}

impl ProxyKey {
    pub(super) fn of(proxy: &ProxyUrl) -> Option<ProxyKey> {
        let url = parse(proxy)?;
        Some(ProxyKey {
            scheme: url.scheme().to_owned(),
            username: url.username().to_owned(),
            host: url.host_str().map(str::to_owned),
            port: url.port_or_known_default(),
        })
    }
}

impl Device {
    pub(super) fn resolved_proxy(&self) -> Result<Option<ProxyUrl>> {
        let (Some(proxy), Some(var)) = (&self.proxy, &self.proxy_password_env) else {
            return Ok(self.proxy.clone());
        };
        let password = std::env::var(var).map_err(|_| {
            config(format!(
                "the proxy password variable {var} is not set or not valid UTF-8"
            ))
        })?;
        let mut url = parse(proxy).ok_or_else(|| config("the device proxy URL is invalid"))?;
        url.set_password(Some(&password))
            .map_err(|()| config("the device proxy URL cannot carry a password"))?;
        ProxyUrl::parse(url.as_str()).map(Some)
    }

    pub(super) fn sanitized(&self) -> Device {
        let mut device = self.clone();
        if device.proxy_password_env.is_some() {
            device.proxy = device.proxy.as_ref().map(strip_password);
        }
        device
    }
}

fn strip_password(proxy: &ProxyUrl) -> ProxyUrl {
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
