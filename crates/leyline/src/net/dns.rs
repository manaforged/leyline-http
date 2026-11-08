use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use super::resolver::{Coalesced, ResolveFuture, Resolver, SystemResolver};

#[derive(Clone)]
pub(crate) struct DnsConfig {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
}

impl PartialEq for DnsConfig {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.resolver, &other.resolver) && self.overrides == other.overrides
    }
}

impl std::fmt::Debug for DnsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsConfig")
            .field("overrides", &self.overrides)
            .finish_non_exhaustive()
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            resolver: Arc::new(SystemResolver),
            overrides: HashMap::new(),
        }
    }
}

impl DnsConfig {
    pub(crate) fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub(crate) fn resolve_host(
        mut self,
        host: impl AsRef<str>,
        addrs: impl IntoIterator<Item = SocketAddr>,
    ) -> Self {
        let host = normalize_host(host.as_ref());
        let addrs: Vec<SocketAddr> = addrs.into_iter().collect();
        if addrs.is_empty() {
            self.overrides.remove(&host);
        } else {
            self.overrides.insert(host, addrs);
        }
        self
    }

    pub(crate) fn into_resolver(self) -> Arc<dyn Resolver> {
        let resolver = Arc::new(Coalesced::new(self.resolver));
        if self.overrides.is_empty() {
            resolver
        } else {
            Arc::new(LayeredResolver {
                resolver,
                overrides: self.overrides,
            })
        }
    }
}

struct LayeredResolver {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
}

impl Resolver for LayeredResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        let key = normalize_host(host);
        if let Some(addrs) = self.overrides.get(&key) {
            let addrs = addrs
                .iter()
                .map(|addr| SocketAddr::new(addr.ip(), port))
                .collect::<Vec<_>>();
            return Box::pin(async move { Ok(addrs) });
        }
        self.resolver.resolve(host, port)
    }
}

pub(crate) fn normalize_host(host: &str) -> String {
    let stripped = host.trim().trim_end_matches('.');
    let stripped = stripped
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stripped);
    let lowered = stripped.to_ascii_lowercase();
    match url::Host::parse(&lowered) {
        Ok(url::Host::Domain(d)) => d,
        Ok(url::Host::Ipv4(a)) => a.to_string(),
        Ok(url::Host::Ipv6(a)) => a.to_string(),
        Err(_) => lowered,
    }
}
