use std::io;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lru::LruCache;
use url::Url;

use crate::core::Session;
use crate::error::{Error, Kind};
use crate::net::dns_wire::{
    DnsAnswer, TYPE_A, TYPE_AAAA, TYPE_HTTPS, WireError, encode_query, parse_response,
};
use crate::net::{EchFuture, ResolveFuture, Resolver, normalize_host};
use crate::util::lock;

const DNS_MESSAGE: &str = "application/dns-message";
const CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(256).unwrap();
pub(crate) const MAX_DOH_RESPONSE_BYTES: usize = 65_535;
const MIN_TTL: Duration = Duration::from_secs(1);
const MAX_TTL: Duration = Duration::from_secs(3600);
const DEFAULT_TTL: Duration = Duration::from_secs(60);
const NEGATIVE_ECH_TTL: Duration = Duration::from_secs(60);

struct CachedIps {
    addrs: Vec<IpAddr>,
    expires: Instant,
}

struct CachedEch {
    config: Option<Vec<u8>>,
    expires: Instant,
}

pub(crate) struct DohResolver {
    endpoint: Url,
    client: Session,
    ip_cache: Arc<Mutex<LruCache<String, CachedIps>>>,
    ech_cache: Arc<Mutex<LruCache<(String, u16), CachedEch>>>,
}

impl DohResolver {
    pub(crate) fn new(endpoint: Url, client: Session) -> Self {
        Self {
            endpoint,
            client,
            ip_cache: Arc::new(Mutex::new(LruCache::new(CACHE_CAPACITY))),
            ech_cache: Arc::new(Mutex::new(LruCache::new(CACHE_CAPACITY))),
        }
    }

    pub(crate) fn per_core(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            client: self.client.per_core(),
            ip_cache: Arc::clone(&self.ip_cache),
            ech_cache: Arc::clone(&self.ech_cache),
        }
    }

    async fn query(&self, host: &str, qtype: u16) -> Result<DnsAnswer, io::Error> {
        let wire = encode_query(host, qtype).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("dns query: {e:?}"))
        })?;
        let resp = self
            .client
            .post(self.endpoint.as_str())
            .header("content-type", DNS_MESSAGE)
            .header("accept", DNS_MESSAGE)
            .body(wire)
            .send()
            .await
            .map_err(|e| doh_io_error("doh request failed", &e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(io::Error::other(format!("doh server returned {status}")));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| doh_io_error("doh response read failed", &e))?;
        parse_response(&body).map_err(wire_to_io)
    }

    fn cached_ips(&self, key: &str, port: u16) -> Option<Vec<SocketAddr>> {
        let mut cache = lock(&self.ip_cache);
        let entry = cache.get(key)?;
        if Instant::now() < entry.expires {
            return Some(
                entry
                    .addrs
                    .iter()
                    .map(|&ip| SocketAddr::new(ip, port))
                    .collect(),
            );
        }
        cache.pop(key);
        None
    }

    fn store_ips(&self, key: String, addrs: Vec<IpAddr>, ttl: Option<u32>) {
        let expires = Instant::now() + clamp_ttl(ttl, DEFAULT_TTL);
        lock(&self.ip_cache).put(key, CachedIps { addrs, expires });
    }

    fn cached_ech(&self, key: &(String, u16)) -> Option<Option<Vec<u8>>> {
        let mut cache = lock(&self.ech_cache);
        let entry = cache.get(key)?;
        if Instant::now() < entry.expires {
            return Some(entry.config.clone());
        }
        cache.pop(key);
        None
    }

    fn store_ech(&self, key: (String, u16), config: Option<Vec<u8>>, ttl: Option<u32>) {
        let fallback = if config.is_some() {
            DEFAULT_TTL
        } else {
            NEGATIVE_ECH_TTL
        };
        let expires = Instant::now() + clamp_ttl(ttl, fallback);
        lock(&self.ech_cache).put(key, CachedEch { config, expires });
    }
}

fn doh_io_error(context: &str, err: &Error) -> io::Error {
    let kind = if err.is_timeout() {
        io::ErrorKind::TimedOut
    } else if err.kind() == Kind::Config {
        io::ErrorKind::InvalidInput
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, format!("{context}: {err}"))
}

fn clamp_ttl(ttl: Option<u32>, fallback: Duration) -> Duration {
    ttl.map(|secs| Duration::from_secs(u64::from(secs)))
        .unwrap_or(fallback)
        .clamp(MIN_TTL, MAX_TTL)
}

fn wire_to_io(err: WireError) -> io::Error {
    match err {
        WireError::NxDomain => io::Error::new(io::ErrorKind::NotFound, "dns NXDOMAIN"),
        other => io::Error::new(io::ErrorKind::InvalidData, format!("dns wire: {other:?}")),
    }
}

impl Resolver for DohResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        if let Some(ip) = crate::util::parse_ip_host(host) {
            return Box::pin(async move { Ok(vec![SocketAddr::new(ip, port)]) });
        }
        let key = normalize_host(host);
        Box::pin(async move {
            if let Some(addrs) = self.cached_ips(&key, port) {
                return Ok(addrs);
            }
            let (v4, v6) = tokio::join!(self.query(&key, TYPE_A), self.query(&key, TYPE_AAAA));
            let mut addrs = Vec::new();
            let mut min_ttl: Option<u32> = None;
            let mut last_err: Option<io::Error> = None;
            for result in [v4, v6] {
                match result {
                    Ok(answer) => {
                        addrs.extend(answer.addrs);
                        if let Some(ttl) = answer.min_ttl {
                            crate::net::dns_wire::update_min_ttl(&mut min_ttl, ttl);
                        }
                    }
                    Err(err) => last_err = Some(err),
                }
            }
            if addrs.is_empty() {
                return Err(last_err.unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, format!("no DNS records for {key}"))
                }));
            }
            let out = addrs
                .iter()
                .map(|&ip| SocketAddr::new(ip, port))
                .collect::<Vec<_>>();
            self.store_ips(key, addrs, min_ttl);
            Ok(out)
        })
    }

    fn resolve_ech<'a>(&'a self, host: &'a str, port: u16) -> EchFuture<'a> {
        if crate::util::parse_ip_host(host).is_some() {
            return Box::pin(async { None });
        }
        let host = normalize_host(host);
        Box::pin(async move {
            let cache_key = (host, port);
            if let Some(cached) = self.cached_ech(&cache_key) {
                return cached;
            }
            let qname = if port == 443 {
                cache_key.0.clone()
            } else {
                format!("_{port}._https.{}", cache_key.0)
            };
            let Ok(answer) = self.query(&qname, TYPE_HTTPS).await else {
                return None;
            };
            let config = answer.ech_config_list;
            self.store_ech(cache_key, config.clone(), answer.min_ttl);
            config
        })
    }
}
