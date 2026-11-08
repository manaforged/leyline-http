use crate::error::{Error, Result};
use crate::util::redact;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProxyScheme {
    Http,
    Https,
    Socks5,
    Socks5h,
}

impl ProxyScheme {
    fn parse(scheme: &str) -> Option<Self> {
        match scheme {
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            "socks5" => Some(Self::Socks5),
            "socks5h" => Some(Self::Socks5h),
            _ => None,
        }
    }

    pub(crate) fn is_socks(self) -> bool {
        matches!(self, Self::Socks5 | Self::Socks5h)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
/// A validated proxy URL with its text as written.
pub struct ProxyUrl {
    raw: String,
    url: url::Url,
    scheme: ProxyScheme,
}

impl TryFrom<String> for ProxyUrl {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self> {
        Self::parse(raw)
    }
}

impl std::fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProxyUrl").field(&redact(&self.raw)).finish()
    }
}

impl ProxyUrl {
    /// Parses an http, https, socks5, or socks5h proxy URL with a host.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self> {
        let raw = raw.as_ref().trim();
        let url =
            url::Url::parse(raw).map_err(|e| Error::config(format!("invalid proxy URL: {e}")))?;
        let Some(scheme) = ProxyScheme::parse(url.scheme()) else {
            return Err(Error::config(format!(
                "unsupported proxy scheme {:?}; expected http, https, socks5, or socks5h",
                url.scheme()
            )));
        };
        if url.host_str().is_none() {
            return Err(Error::config("proxy URL must include a host"));
        }
        Ok(Self {
            raw: raw.to_string(),
            url,
            scheme,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.raw
    }

    pub(crate) fn url(&self) -> &url::Url {
        &self.url
    }

    pub(crate) fn scheme(&self) -> ProxyScheme {
        self.scheme
    }

    pub(crate) fn with_password(&self, password: &str) -> Result<Self> {
        let mut url = self.url.clone();
        url.set_password(Some(password))
            .map_err(|()| Error::config("the proxy URL cannot carry a password"))?;
        Ok(Self {
            raw: url.to_string(),
            url,
            scheme: self.scheme,
        })
    }

    pub(crate) fn without_password(&self) -> Self {
        let mut url = self.url.clone();
        if self.url.password().is_none() || url.set_password(None).is_err() {
            return self.clone();
        }
        Self {
            raw: url.to_string(),
            url,
            scheme: self.scheme,
        }
    }
}

/// The error text for a SOCKS5 proxy in a build without the `socks` feature.
#[cfg(any(not(feature = "socks"), feature = "http3"))]
pub(crate) const SOCKS_FEATURE_OFF: &str = "a SOCKS5 proxy needs the `socks` feature of \
                                            leyline-http: enable `features = [\"socks\"]`";

/// Returns the error text for the SOCKS proxy `raw` in a build without the `socks` feature.
#[cfg(not(feature = "socks"))]
pub(crate) fn socks_disabled(raw: &str) -> String {
    format!("proxy {}: {SOCKS_FEATURE_OFF}", redact(raw))
}

impl std::fmt::Display for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&redact(&self.raw))
    }
}

impl From<ProxyUrl> for String {
    fn from(value: ProxyUrl) -> Self {
        value.raw
    }
}
