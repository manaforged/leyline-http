use std::sync::Mutex;
use std::time::SystemTime;

use http::header::{self, HeaderName};
use http::{HeaderMap, Method, Request, StatusCode};
use http_cache_semantics::{AfterResponse, BeforeRequest, CacheOptions, CachePolicy};
use lru::LruCache;
use sha2::{Digest, Sha256};

use super::Session;
use super::header_merge::apply_extra_headers;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::transport::{Prepared, ResponseMode, TransportBody, TransportResponse};
use crate::util::lock;

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
    tls: Option<crate::pool::TlsInfo>,
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
        let proxy = request.proxy.map_or("", |proxy| proxy.url().as_str());
        hash.update((proxy.len() as u64).to_be_bytes());
        hash.update(proxy.as_bytes());
        for (name, value) in &request.headers {
            if [header::CACHE_CONTROL, header::PRAGMA]
                .iter()
                .any(|skip| name.eq_ignore_ascii_case(skip.as_str()))
            {
                continue;
            }
            for bytes in [name.as_bytes(), value.as_str().as_bytes()] {
                hash.update((bytes.len() as u64).to_be_bytes());
                hash.update(bytes);
            }
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
    ) -> Result<TransportResponse> {
        let request = request(&req)?;
        let key = Key::of(&req);
        if !eligible(&request, &req) {
            if only_cached(&request) {
                return Ok(cache_miss());
            }
            let response = self.send_transport(req).await?;
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
        let response = self.send_transport(req).await?;
        lookup.entry = stale;
        finish(cache, key, &request, response, lookup)
    }

    fn invalidate(&self, cache: &Cache, request: &Request<()>, response: &TransportResponse) {
        if matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
        ) || !(response.status.is_success() || response.status.is_redirection())
        {
            return;
        }
        cache.remove(&request.uri().to_string());
        let Ok(base) = url::Url::parse(&request.uri().to_string()) else {
            return;
        };
        for name in [header::LOCATION, header::CONTENT_LOCATION] {
            if let Some(to) = response
                .headers
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
    apply_extra_headers(&mut req.headers, &extra, false, &|_| false);
    Ok(())
}

fn only_cached(request: &Request<()>) -> bool {
    request
        .headers()
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|value| value.trim().eq_ignore_ascii_case("only-if-cached"))
}

fn finish(
    cache: &Cache,
    key: Key,
    request: &Request<()>,
    mut response: TransportResponse,
    lookup: Lookup,
) -> Result<TransportResponse> {
    let mut headers = response.headers.clone();
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
                response.timing = timing;
                for name in VOLATILE {
                    for value in actual.get_all(&name) {
                        response.headers.append(name.clone(), value.clone());
                    }
                }
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
    if response.headers.contains_key(header::SET_COOKIE) {
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
            .saturating_add(header_cost(&response.headers))
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

    fn response(&self, parts: http::response::Parts) -> TransportResponse {
        TransportResponse {
            status: parts.status,
            headers: parts.headers,
            trailers: None,
            body: TransportBody::Buffered(self.body.clone()),
            version: self.version,
            tls: self.tls.clone(),
            timing: Default::default(),
        }
    }
}

fn cache_miss() -> TransportResponse {
    TransportResponse {
        status: StatusCode::GATEWAY_TIMEOUT,
        headers: HeaderMap::new(),
        trailers: None,
        body: TransportBody::Buffered(Vec::new()),
        version: http::Version::HTTP_11,
        tls: None,
        timing: Default::default(),
    }
}
