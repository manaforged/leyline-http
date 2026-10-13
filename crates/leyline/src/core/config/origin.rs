use super::host::normalize_host;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Origin {
    scheme: String,
    pub(super) host: String,
    port: u16,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.host.contains(':') {
            true => write!(f, "{}://[{}]:{}", self.scheme, self.host, self.port),
            false => write!(f, "{}://{}:{}", self.scheme, self.host, self.port),
        }
    }
}

impl Origin {
    pub(crate) fn of(url: &url::Url) -> Option<Self> {
        Some(Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: normalize_host(url.host_str()?),
            port: url.port_or_known_default()?,
        })
    }
}
