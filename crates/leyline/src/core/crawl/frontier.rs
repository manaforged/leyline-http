use std::collections::{HashMap, HashSet, VecDeque};

use tokio::time::Instant;
use url::Url;

use crate::cookie::parse::registrable_domain;
use crate::core::robots::{Robots, normalize};

const WEB_SCHEMES: [&str; 2] = ["http", "https"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Scope {
    #[default]
    Host,
    Domain,
    Prefix,
}

enum Bound {
    Host(String),
    Domain(String),
    Prefix { host: String, path: String },
}

fn host(url: &Url) -> Option<String> {
    Some(url.host_str()?.trim_end_matches('.').to_ascii_lowercase())
}

impl Bound {
    fn of(scope: Scope, url: &Url) -> Option<Bound> {
        let host = host(url)?;
        Some(match scope {
            Scope::Host => Bound::Host(host),
            Scope::Domain => Bound::Domain(registrable_domain(&host).unwrap_or(host)),
            Scope::Prefix => {
                let path = url.path();
                let dir = path.rfind('/').map_or("/", |end| &path[..=end]);
                Bound::Prefix {
                    host,
                    path: dir.to_owned(),
                }
            }
        })
    }

    fn holds(&self, url: &Url) -> bool {
        let Some(candidate) = host(url) else {
            return false;
        };
        match self {
            Bound::Host(bound) => candidate == *bound,
            Bound::Domain(domain) => registrable_domain(&candidate).unwrap_or(candidate) == *domain,
            Bound::Prefix { host, path } => candidate == *host && url.path().starts_with(path),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Queued {
    pub(super) url: Url,
    pub(super) depth: usize,
    pub(super) hops: usize,
    pub(super) seed: bool,
}

pub(super) enum Site {
    Pending(Vec<Queued>),
    Ready(Robots, Instant),
}

#[derive(Default)]
pub(super) struct Frontier {
    queue: VecDeque<Queued>,
    seen: HashSet<String>,
    bounds: Vec<Bound>,
    pub(super) scope: Scope,
    pub(super) sites: HashMap<String, Site>,
}

pub(super) fn site_key(url: &Url) -> String {
    url.origin().ascii_serialization()
}

pub(super) fn page_key(url: &Url) -> String {
    let mut key = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
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
    pub(super) fn add(&mut self, mut queued: Queued) {
        queued.url.set_fragment(None);
        if !WEB_SCHEMES.contains(&queued.url.scheme()) {
            return;
        }
        if queued.seed
            && let Some(bound) = Bound::of(self.scope, &queued.url)
        {
            self.bounds.push(bound);
        }
        if !self.in_scope(&queued.url) {
            return;
        }
        if self.seen.insert(page_key(&queued.url)) {
            self.queue.push_back(queued);
        }
    }

    pub(super) fn in_scope(&self, url: &Url) -> bool {
        self.bounds.iter().any(|bound| bound.holds(url))
    }

    pub(super) fn known(&self) -> usize {
        self.seen.len()
    }

    pub(super) fn pop(&mut self) -> Option<Queued> {
        self.queue.pop_front()
    }

    pub(super) fn resume(&mut self, waiting: Vec<Queued>) {
        for queued in waiting.into_iter().rev() {
            self.queue.push_front(queued);
        }
    }

    pub(super) fn clear(&mut self) {
        self.queue.clear();
    }
}
