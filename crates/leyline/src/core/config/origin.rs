use crate::wire::Scheme;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

const TUPLE_SCHEMES: [&str; 5] = ["http", "https", "ws", "wss", "ftp"];

impl Origin {
    pub(crate) fn same(a: &url::Url, b: &url::Url) -> bool {
        if a.scheme() == "blob" || b.scheme() == "blob" {
            return a.origin() == b.origin();
        }
        TUPLE_SCHEMES.contains(&a.scheme())
            && (std::ptr::eq(a, b)
                || a.scheme() == b.scheme()
                    && a.host() == b.host()
                    && a.port_or_known_default() == b.port_or_known_default())
    }

    pub(crate) fn downgrades(from: &url::Url, to: &url::Url) -> bool {
        let secure = |url: &url::Url| Scheme::of(url).is_some_and(Scheme::is_secure);
        secure(from) && !secure(to)
    }

    pub(crate) fn ascii(url: &url::Url) -> std::borrow::Cow<'_, str> {
        if url.has_authority() && url.username().is_empty() && url.password().is_none() {
            return std::borrow::Cow::Borrowed(&url[..url::Position::BeforePath]);
        }
        let host = url.host_str().unwrap_or("");
        std::borrow::Cow::Owned(match url.port() {
            Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
            None => format!("{}://{}", url.scheme(), host),
        })
    }
}
