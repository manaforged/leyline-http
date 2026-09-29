use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::tls::{Resolver, SystemResolver};

use super::host::normalize_host;

#[derive(Clone)]
#[non_exhaustive]
pub struct DnsConfig {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub fn resolve_host(
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
        if self.overrides.is_empty() {
            self.resolver
        } else {
            Arc::new(LayeredResolver {
                resolver: self.resolver,
                overrides: self.overrides,
            })
        }
    }
}

impl From<Arc<dyn Resolver>> for DnsConfig {
    fn from(resolver: Arc<dyn Resolver>) -> Self {
        Self::new().resolver(resolver)
    }
}

struct LayeredResolver {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
}

impl Resolver for LayeredResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> crate::tls::ResolveFuture<'a> {
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
