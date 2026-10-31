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
            host: match url.host()? {
                url::Host::Domain(domain) => domain.trim_end_matches('.').to_ascii_lowercase(),
                url::Host::Ipv4(addr) => addr.to_string(),
                url::Host::Ipv6(addr) => addr.to_string(),
            },
            port: url.port_or_known_default()?,
        })
    }
}
