use std::collections::{HashMap, HashSet, VecDeque};

use url::{Host, Url};

use crate::core::config::Origin;
use crate::core::robots::normalize;
use crate::net::normalize_host;
use crate::util::registrable_domain;
use crate::wire::Scheme;

mod queues;

const DEFAULT_MAX_URL_BYTES: usize = 2048;
const DEFAULT_MAX_DEPTH: usize = 64;

fn default_max_url_size() -> Option<usize> {
    Some(DEFAULT_MAX_URL_BYTES)
}

fn default_max_depth() -> Option<usize> {
    Some(DEFAULT_MAX_DEPTH)
}

/// Which URLs a crawl follows, relative to the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Scope {
    /// Follows URLs on the seed's exact host; the default.
    #[default]
    Host,
    /// Follows URLs on the seed's registrable domain and its subdomains, or the exact host for an IP address.
    Domain,
    /// Follows URLs on the seed's host under the seed's directory path.
    Prefix,
    /// Follows URLs on any host.
    Any,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Bound {
    Host(String),
    Domain(String),
    Prefix { host: String, dir: String },
    Any,
}

fn host(url: &Url) -> Option<String> {
    Some(normalize_host(url.host_str()?))
}

fn is_ip(url: &Url) -> bool {
    matches!(url.host(), Some(Host::Ipv4(_) | Host::Ipv6(_)))
}

fn directory(path: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or_default();
    let named_file = last
        .rsplit_once('.')
        .is_some_and(|(_, ext)| ext.bytes().any(|byte| byte.is_ascii_alphabetic()));
    if named_file {
        let end = path.rfind('/').map_or(0, |end| end + 1);
        return path[..end].to_owned();
    }
    format!("{}/", path.trim_end_matches('/'))
}

impl Bound {
    fn of(scope: Scope, url: &Url) -> Option<Bound> {
        let host = host(url)?;
        Some(match scope {
            Scope::Domain if !is_ip(url) => {
                Bound::Domain(registrable_domain(&host).unwrap_or(host))
            }
            Scope::Host | Scope::Domain => Bound::Host(host),
            Scope::Prefix => Bound::Prefix {
                host,
                dir: directory(url.path()),
            },
            Scope::Any => Bound::Any,
        })
    }

    fn holds(&self, url: &Url) -> bool {
        let Some(candidate) = host(url) else {
            return false;
        };
        match self {
            Bound::Host(bound) => candidate == *bound,
            Bound::Domain(domain) => {
                !is_ip(url) && registrable_domain(&candidate).unwrap_or(candidate) == *domain
            }
            Bound::Prefix { host, dir } => {
                let path = url.path();
                candidate == *host
                    && (path.starts_with(dir.as_str()) || path == dir.trim_end_matches('/'))
            }
            Bound::Any => true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Queued {
    pub(super) url: Url,
    pub(super) depth: usize,
    pub(super) hops: usize,
    pub(super) seed: bool,
    pub(super) widen: bool,
    pub(super) attempts: u32,
    pub(super) chain: Vec<String>,
    #[serde(default)]
    pub(super) previous: Vec<Url>,
}

impl Queued {
    pub(super) fn link(url: Url, depth: usize) -> Queued {
        Queued {
            url,
            depth,
            hops: 0,
            seed: false,
            widen: false,
            attempts: 0,
            chain: Vec::new(),
            previous: Vec::new(),
        }
    }

    pub(super) fn seed(url: Url) -> Queued {
        Queued {
            seed: true,
            widen: true,
            ..Queued::link(url, 0)
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Frontier {
    #[serde(with = "queues")]
    queues: HashMap<Origin, VecDeque<Queued>>,
    order: VecDeque<Origin>,
    seen: HashSet<String>,
    redirected: HashSet<String>,
    bounds: Vec<Bound>,
    pub(super) scope: Scope,
    #[serde(default = "default_max_depth")]
    pub(super) max_depth: Option<usize>,
    #[serde(default = "default_max_url_size")]
    pub(super) max_url_size: Option<usize>,
}

impl Default for Frontier {
    fn default() -> Self {
        Self {
            queues: HashMap::new(),
            order: VecDeque::new(),
            seen: HashSet::new(),
            redirected: HashSet::new(),
            bounds: Vec::new(),
            scope: Scope::default(),
            max_depth: default_max_depth(),
            max_url_size: default_max_url_size(),
        }
    }
}

pub(super) fn same_site(from: &Url, to: &Url) -> bool {
    match (Bound::of(Scope::Domain, from), Bound::of(Scope::Domain, to)) {
        (Some(Bound::Domain(from)), Some(Bound::Domain(to))) => from == to,
        _ => host(from) == host(to),
    }
}

pub(super) fn page_key(url: &Url) -> String {
    let mut key = format!("{}://{}", url.scheme(), host(url).unwrap_or_default());
    if let Some(port) = url.port() {
        key.push_str(&format!(":{port}"));
    }
    key.push_str(&normalize(url.path()));
    if let Some(query) = url.query() {
        let mut pairs: Vec<String> = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(normalize)
            .collect();
        pairs.sort_unstable();
        key.push('?');
        key.push_str(&pairs.join("&"));
    }
    key
}

impl Frontier {
    pub(super) fn add(&mut self, queued: Queued) -> bool {
        self.admit(queued, false)
    }

    pub(super) fn add_redirect(&mut self, queued: Queued) -> bool {
        self.admit(queued, true)
    }

    fn admit(&mut self, mut queued: Queued, redirect: bool) -> bool {
        queued.url.set_fragment(None);
        let within_size = self
            .max_url_size
            .is_none_or(|max| queued.url.as_str().len() <= max);
        let within_depth = self.max_depth.is_none_or(|max| queued.depth <= max);
        let usable = Scheme::of(&queued.url).is_some() && within_size && within_depth;
        if !usable {
            return false;
        }
        if queued.widen
            && let Some(bound) = Bound::of(self.scope, &queued.url)
        {
            self.bounds.push(bound);
        }
        if !queued.seed && !self.in_scope(&queued.url) {
            return false;
        }
        let key = page_key(&queued.url);
        let again = redirect && self.redirected.remove(&key);
        if !self.seen.insert(key) && !again {
            return false;
        }
        self.push_back(queued);
        true
    }

    pub(super) fn rekey(&mut self, mut queued: Queued, to: Url) {
        self.seen.remove(&page_key(&queued.url));
        queued.url = to;
        self.add(queued);
    }

    pub(super) fn has_seen(&self, url: &Url) -> bool {
        self.seen.contains(&page_key(url))
    }

    pub(super) fn redirected(&mut self, url: &Url) {
        self.redirected.insert(page_key(url));
    }

    pub(super) fn in_scope(&self, url: &Url) -> bool {
        self.bounds.iter().any(|bound| bound.holds(url))
    }

    pub(super) fn validate(&self) -> crate::error::Result<()> {
        let order: HashSet<_> = self.order.iter().collect();
        if order.len() != self.order.len()
            || order.len() != self.queues.len()
            || self.queues.keys().any(|origin| !order.contains(origin))
            || self.queues.values().flatten().any(|queued| {
                self.max_url_size
                    .is_some_and(|max| queued.url.as_str().len() > max)
                    || self.max_depth.is_some_and(|max| queued.depth > max)
                    || Scheme::of(&queued.url).is_none()
                    || !self.has_seen(&queued.url)
                    || (!queued.seed && !self.in_scope(&queued.url))
            })
        {
            return Err(crate::error::Error::config(
                "checkpoint contains an inconsistent crawl frontier",
            ));
        }
        Ok(())
    }

    pub(super) fn known(&self) -> usize {
        self.seen.len()
    }

    pub(super) fn push_back(&mut self, queued: Queued) {
        let Some(origin) = Origin::of(&queued.url) else {
            return;
        };
        let queue = self.queues.entry(origin.clone()).or_default();
        if queue.is_empty() {
            self.order.push_back(origin);
        }
        queue.push_back(queued);
    }

    pub(super) fn push_front(&mut self, queued: Queued) {
        let Some(origin) = Origin::of(&queued.url) else {
            return;
        };
        let queue = self.queues.entry(origin.clone()).or_default();
        if queue.is_empty() {
            self.order.push_front(origin);
        }
        queue.push_front(queued);
    }

    pub(super) fn pop_where(&mut self, mut ready: impl FnMut(&Origin) -> bool) -> Option<Queued> {
        for _ in 0..self.order.len() {
            let origin = self.order.pop_front()?;
            if !ready(&origin) {
                self.order.push_back(origin);
                continue;
            }
            let queue = self.queues.get_mut(&origin)?;
            let queued = queue.pop_front();
            if queue.is_empty() {
                self.queues.remove(&origin);
            } else {
                self.order.push_back(origin);
            }
            return queued;
        }
        None
    }

    pub(super) fn origins(&self) -> impl Iterator<Item = &Origin> {
        self.queues.keys()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.queues.clear();
        self.order.clear();
    }
}
