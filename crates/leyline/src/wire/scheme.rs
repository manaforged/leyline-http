/// The scheme of a URL that carries HTTP.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub(crate) fn of(url: &url::Url) -> Option<Self> {
        Self::parse(url.scheme())
    }

    pub(crate) fn parse(scheme: &str) -> Option<Self> {
        match scheme {
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    pub(crate) const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }

    pub(crate) const fn is_secure(self) -> bool {
        matches!(self, Self::Https)
    }
}

pub(crate) fn authority<'a>(
    url: &'a url::Url,
    host: &'a str,
    port: u16,
) -> std::borrow::Cow<'a, str> {
    let is_default_port = Scheme::of(url).is_some_and(|scheme| scheme.default_port() == port);
    if is_default_port {
        std::borrow::Cow::Borrowed(host)
    } else if url.port() == Some(port) && url.host_str() == Some(host) {
        std::borrow::Cow::Borrowed(&url[url::Position::BeforeHost..url::Position::AfterPort])
    } else {
        std::borrow::Cow::Owned(format!("{host}:{port}"))
    }
}

#[inline]
pub(crate) fn host_port(url: &url::Url) -> crate::error::Result<(&str, u16)> {
    match (url.host_str(), url.port_or_known_default()) {
        (Some(host), Some(port)) => Ok((host, port)),
        _ => Err(no_host_port(url)),
    }
}

#[cold]
fn no_host_port(url: &url::Url) -> crate::error::Error {
    if url.host_str().is_none() {
        return crate::error::Error::config("no host in URL");
    }
    crate::error::Error::config(format!("no default port for scheme {}", url.scheme()))
}
