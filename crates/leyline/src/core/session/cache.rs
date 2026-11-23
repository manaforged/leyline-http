use std::sync::Mutex;
use std::time::SystemTime;

use http::header::{self, HeaderName};
use http::{HeaderMap, Method, Request, StatusCode};
use http_cache_semantics::{AfterResponse, BeforeRequest, CacheOptions, CachePolicy};
use lru::LruCache;
use sha2::{Digest, Sha256};

use super::header_merge::apply_extra_headers;
use super::{Protocol, Session};
use crate::core::headers::HeaderList;
use crate::error::{Error, Kind, Result};
use crate::pool::Prepared;
use crate::util::lock;
use crate::wire::{ResponseMode, ResponseTiming, TransportBody, TransportResponse};

pub(super) struct Cache {
    capacity: usize,
    state: Mutex<Store>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    url: String,
    context: [u8; 32],
}

struct Store {
    entries: LruCache<Key, std::sync::Arc<Entry>>,
    bytes: usize,
    generation: u64,
}

struct Lookup {
    entry: Option<std::sync::Arc<Entry>>,
    generation: u64,
}

struct Entry {
    policy: CachePolicy,
    body: Vec<u8>,
    version: http::Version,
    tls: Option<crate::wire::TlsInfo>,
    cost: usize,
}

const RANGE_HEADERS: [HeaderName; 2] = [header::RANGE, header::IF_RANGE];
const CONDITIONS: [HeaderName; 2] = [header::IF_NONE_MATCH, header::IF_MODIFIED_SINCE];
const VOLATILE: [HeaderName; 3] = [
    header::SET_COOKIE,
    header::STRICT_TRANSPORT_SECURITY,
    HeaderName::from_static("alt-svc"),
];

impl Key {
    fn of(request: &Prepared<'_>) -> Self {
        let mut hash = Sha256::new();
        let proxy = request
            .proxy
            .map(crate::net::ProxyUrl::identity)
            .unwrap_or_default();
        hash.update((proxy.len() as u64).to_be_bytes());
        hash.update(proxy.as_bytes());
        for (name, value) in &request.headers {
            if [header::CACHE_CONTROL, header::PRAGMA]
                .iter()
                .any(|skip| name.eq_ignore_ascii_case(skip.as_str()))
            {
                continue;
            }
            hash.update((name.len() as u64).to_be_bytes());
            for byte in name.as_bytes() {
                hash.update([byte.to_ascii_lowercase()]);
            }
            let val = value.as_str().as_bytes();
            hash.update((val.len() as u64).to_be_bytes());
            hash.update(val);
        }
        Self {
            url: request.url[..url::Position::AfterQuery].to_owned(),
            context: hash.finalize().into(),
        }
    }
}

impl Cache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(Store {
                entries: LruCache::unbounded(),
                bytes: 0,
                generation: 0,
            }),
        }
    }

    fn lookup(&self, key: &Key) -> Lookup {
        let mut state = lock(&self.state);
        Lookup {
            entry: state.entries.get(key).cloned(),
            generation: state.generation,
        }
    }

    fn remove(&self, key: &str) {
        let mut state = lock(&self.state);
        state.generation = state.generation.wrapping_add(1);
        let keys: Vec<_> = state
            .entries
            .iter()
            .filter(|(entry, _)| entry.url == key)
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            if let Some(entry) = state.entries.pop(&key) {
                state.bytes -= entry.cost;
            }
        }
    }

    fn store(&self, key: Key, entry: Entry, generation: u64) {
        let mut state = lock(&self.state);
        if state.generation != generation {
            return;
        }
        if let Some(old) = state.entries.pop(&key) {
            state.bytes -= old.cost;
        }
        if !entry.policy.is_storable() || entry.cost > self.capacity {
            return;
        }
        while state.bytes > self.capacity - entry.cost {
            let Some((_, old)) = state.entries.pop_lru() else {
                break;
            };
            state.bytes -= old.cost;
        }
        state.bytes += entry.cost;
        state.entries.put(key, std::sync::Arc::new(entry));
    }
}

impl Session {
    /// Removes every response stored in this session's shared HTTP cache.
    pub fn clear_cache(&self) {
        if let Some(cache) = &self.inner.cache {
            let mut state = lock(&cache.state);
            state.entries.clear();
            state.bytes = 0;
            state.generation = state.generation.wrapping_add(1);
        }
    }

    pub(super) async fn send_cached(
        &self,
        mut req: Prepared<'_>,
        cache: &Cache,
        protocol: Protocol,
    ) -> Result<Box<TransportResponse>> {
        let request = request(&req)?;
        let key = Key::of(&req);
        if !eligible(&request, &req) {
            if only_cached(&request) {
                return Ok(cache_miss());
            }
            let response = self.send_transport(req, protocol).await?;
            self.invalidate(cache, &request, &response);
            return Ok(response);
        }
        let mut lookup = cache.lookup(&key);
        let stale = match lookup.entry.take() {
            Some(entry) => match entry.policy.before_request(&request, SystemTime::now()) {
                BeforeRequest::Fresh(parts) => {
                    tracing::debug!(target: "leyline::cache", "cache hit");
                    return Ok(entry.response(parts));
                }
                BeforeRequest::Stale {
                    request: conditional,
                    matches: true,
                } => {
                    condition(&mut req, &conditional.headers)?;
                    Some(entry)
                }
                BeforeRequest::Stale { matches: false, .. } => None,
            },
            None => None,
        };
        if only_cached(&request) {
            return Ok(cache_miss());
        }
        let response = self.send_transport(req, protocol).await?;
        lookup.entry = stale;
        finish(cache, key, &request, response, lookup)
    }

    pub(super) async fn send_invalidating(
        &self,
        req: Prepared<'_>,
        cache: &Cache,
        protocol: Protocol,
    ) -> Result<Box<TransportResponse>> {
        let method = req.method.clone();
        let uri = req.url[..url::Position::AfterQuery].to_owned();
        let response = self.send_transport(req, protocol).await?;
        self.invalidate_target(cache, &method, &uri, &response);
        Ok(response)
    }

    fn invalidate(&self, cache: &Cache, request: &Request<()>, response: &TransportResponse) {
        self.invalidate_target(
            cache,
            request.method(),
            &request.uri().to_string(),
            response,
        );
    }

    fn invalidate_target(
        &self,
        cache: &Cache,
        method: &Method,
        uri: &str,
        response: &TransportResponse,
    ) {
        if crate::wire::is_safe(method)
            || !(response.status.is_success() || response.status.is_redirection())
        {
            return;
        }
        cache.remove(uri);
        let Ok(base) = url::Url::parse(uri) else {
            return;
        };
        for name in [header::LOCATION, header::CONTENT_LOCATION] {
            if let Some(to) = response
                .headers
                .map()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|value| base.join(value).ok())
                .filter(|to| to.origin() == base.origin())
            {
                cache.remove(&to[..url::Position::AfterQuery]);
            }
        }
    }
}

fn request(req: &Prepared<'_>) -> Result<Request<()>> {
    let mut request = Request::builder()
        .method(req.method)
        .uri(&req.url[..url::Position::AfterQuery]);
    for (name, value) in &req.headers {
        request = request.header(name.as_ref(), value.as_str());
    }
    request
        .body(())
        .map_err(|error| Error::new(Kind::Request).with_source(error))
}

fn eligible(request: &Request<()>, req: &Prepared<'_>) -> bool {
    request.method() == Method::GET
        && req.response != ResponseMode::Streamed
        && req.body.len_hint() == Some(0)
        && req.url.username().is_empty()
        && req.url.password().is_none()
        && RANGE_HEADERS
            .iter()
            .chain(CONDITIONS.iter())
            .all(|name| !request.headers().contains_key(name))
}

fn condition(req: &mut Prepared<'_>, headers: &HeaderMap) -> Result<()> {
    let mut extra = HeaderList::new();
    for name in CONDITIONS {
        for value in headers.get_all(&name) {
            extra.append(name.clone(), value.clone())?;
        }
    }
    apply_extra_headers(req.headers.to_mut(), &extra, false);
    Ok(())
}

fn only_cached(request: &Request<()>) -> bool {
    request
        .headers()
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| crate::util::has_token(v, "only-if-cached"))
}

fn finish(
    cache: &Cache,
    key: Key,
    request: &Request<()>,
    mut response: Box<TransportResponse>,
    lookup: Lookup,
) -> Result<Box<TransportResponse>> {
    let mut headers = response.headers.map().clone();
    for name in VOLATILE {
        headers.remove(name);
    }
    let mut metadata = http::Response::builder()
        .status(response.status)
        .body(())
        .map_err(|error| Error::new(Kind::Request).with_source(error))?;
    *metadata.headers_mut() = headers;
    let policy = match lookup.entry {
        Some(old) => match old
            .policy
            .after_response(request, &metadata, SystemTime::now())
        {
            AfterResponse::NotModified(policy, parts) => {
                let actual = std::mem::take(&mut response.headers);
                let timing = response.timing;
                response = old.response(parts);
                response.timing = timing.cached();
                let mut headers = std::mem::take(&mut response.headers).into_map();
                for name in VOLATILE {
                    for value in actual.map().get_all(&name) {
                        headers.append(name.clone(), value.clone());
                    }
                }
                response.headers = headers.into();
                policy
            }
            AfterResponse::Modified(policy, _) => policy,
        },
        None => CachePolicy::new_options(
            request,
            &metadata,
            SystemTime::now(),
            CacheOptions {
                shared: false,
                ..CacheOptions::default()
            },
        ),
    };
    if response.headers.seen().cookies {
        cache.remove(&key.url);
        return Ok(response);
    }
    match Entry::capture(policy, &key, request, &response, cache.capacity) {
        Some(entry) => cache.store(key, entry, lookup.generation),
        None => cache.remove(&key.url),
    }
    Ok(response)
}

fn header_cost(headers: &HeaderMap) -> usize {
    headers.iter().fold(8192usize, |cost, (name, value)| {
        cost.saturating_add((name.as_str().len() + value.len() + 128).saturating_mul(4))
    })
}

impl Entry {
    fn capture(
        policy: CachePolicy,
        key: &Key,
        request: &Request<()>,
        response: &TransportResponse,
        capacity: usize,
    ) -> Option<Self> {
        let TransportBody::Buffered(body) = &response.body else {
            return None;
        };
        let cost = body
            .len()
            .saturating_add(key.url.len().saturating_mul(4))
            .saturating_add(header_cost(request.headers()))
            .saturating_add(header_cost(response.headers.map()))
            .saturating_add(
                response
                    .tls
                    .as_ref()
                    .and_then(|tls| tls.peer_cert_der.as_ref())
                    .map_or(0, Vec::len),
            );
        if !policy.is_storable() || cost > capacity {
            return None;
        }
        Some(Self {
            policy,
            body: body.clone(),
            version: response.version,
            tls: response.tls.clone(),
            cost,
        })
    }

    fn response(&self, parts: http::response::Parts) -> Box<TransportResponse> {
        Box::new(TransportResponse {
            status: parts.status,
            headers: parts.headers.into(),
            trailers: None,
            body: TransportBody::Buffered(self.body.clone()),
            version: self.version,
            tls: self.tls.clone(),
            timing: ResponseTiming::default().cached(),
        })
    }
}

fn cache_miss() -> Box<TransportResponse> {
    Box::new(TransportResponse {
        status: StatusCode::GATEWAY_TIMEOUT,
        headers: HeaderMap::new().into(),
        trailers: None,
        body: TransportBody::Buffered(Vec::new()),
        version: http::Version::HTTP_11,
        tls: None,
        timing: Default::default(),
    })
}

#[cfg(test)]
mod key_tests {
    use std::sync::Arc;

    use super::*;
    use crate::error::BodyLimit;
    use crate::header_str::HeaderStr;
    use crate::wire::{Body, RequestHeaders, ResponseMode};

    #[test]
    fn proxy_headers_and_request_headers_cannot_collide_in_cache_key() {
        let url = Arc::new(url::Url::parse("https://example.com/resource").unwrap());
        let mut proxy_with_hdr = crate::net::ProxyUrl::parse("http://127.0.0.1:8080").unwrap();
        proxy_with_hdr.extend_headers(&[("X-Tag".into(), "1".into())]);
        let proxy_plain = crate::net::ProxyUrl::parse("http://127.0.0.1:8080").unwrap();

        let req_proxy_hdr = Prepared {
            method: &Method::GET,
            url: &url,
            headers: RequestHeaders::Owned(Vec::new()),
            body: Body::default(),
            proxy: Some(&proxy_with_hdr),
            response: ResponseMode::Buffered,
            body_limit: BodyLimit::session(1024),
            stale_resend: false,
        };
        let req_origin_hdr = Prepared {
            method: &Method::GET,
            url: &url,
            headers: RequestHeaders::Owned(vec![("x-tag".into(), HeaderStr::from("1"))]),
            body: Body::default(),
            proxy: Some(&proxy_plain),
            response: ResponseMode::Buffered,
            body_limit: BodyLimit::session(1024),
            stale_resend: false,
        };

        assert!(Key::of(&req_proxy_hdr) != Key::of(&req_origin_hdr));
    }
}
