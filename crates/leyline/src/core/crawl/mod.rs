mod fetch;
mod frontier;
mod page;
mod sitemap;
mod sites;
mod visit;

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::{FuturesUnordered, Stream, StreamExt};
use url::Url;

use fetch::{Fetched, fetch};
pub use frontier::Scope;
use frontier::{Frontier, Queued};
pub use page::{CrawlPage, CrawlStats, SkipReason};

use crate::core::config::HostLimits;
use crate::core::error::Result;
use crate::core::into_url::IntoUrl;
use crate::core::robots::{Purpose, Robots};
use crate::core::session::Session;
use crate::html::Sitemap;

const ANY_AGENT: &str = "*";
const NOFOLLOW: &str = "nofollow";
const MAX_REDIRECTS: usize = 5;
const MAX_IN_FLIGHT: usize = 16;
const MAX_SITEMAP_FILES: usize = 50;
const POLITE_IN_FLIGHT: usize = 1;
const POLITE_PER_SECOND: f64 = 1.0;
const PAUSE_STATUSES: [u16; 2] = [429, 503];

type Task = Pin<Box<dyn Future<Output = Event> + Send>>;

enum Event {
    Robots {
        site: String,
        robots: Box<Robots>,
    },
    Page {
        queued: Queued,
        fetched: Result<Fetched>,
    },
    Sitemap(Sitemap),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Respect {
    Robots,
    AiTraining,
}

#[must_use = "a crawl is lazy: nothing happens until `.next()` is awaited"]
pub struct Crawl {
    session: Session,
    limits: Option<HostLimits>,
    seeds: Vec<Result<Url>>,
    purpose: Option<Purpose>,
    max_pages: Option<usize>,
    robots: bool,
    sitemap_files: HashSet<String>,
    started: bool,
    frontier: Frontier,
    tasks: FuturesUnordered<Task>,
    pages_in_flight: usize,
    ready: VecDeque<Result<CrawlPage>>,
    kept: HashSet<String>,
    texts: HashSet<u64>,
    stats: CrawlStats,
}

fn polite_limits() -> HostLimits {
    HostLimits::new()
        .max_in_flight(POLITE_IN_FLIGHT)
        .per_second(POLITE_PER_SECOND)
        .pause_on(PAUSE_STATUSES)
}

impl Session {
    pub fn crawl(&self, seed: impl IntoUrl) -> Crawl {
        Crawl {
            session: self.clone(),
            limits: None,
            seeds: vec![seed.into_url_with_base(self.base_url())],
            purpose: None,
            max_pages: None,
            robots: false,
            sitemap_files: HashSet::new(),
            started: false,
            frontier: Frontier::default(),
            tasks: FuturesUnordered::new(),
            pages_in_flight: 0,
            ready: VecDeque::new(),
            kept: HashSet::new(),
            texts: HashSet::new(),
            stats: CrawlStats::default(),
        }
    }
}

impl Crawl {
    pub fn respect(mut self, respect: Respect) -> Self {
        self.robots = true;
        self.purpose = match respect {
            Respect::Robots => None,
            Respect::AiTraining => Some(Purpose::AiTrain),
        };
        self
    }

    pub fn scope(mut self, scope: Scope) -> Self {
        self.frontier.scope = scope;
        self
    }

    pub fn limit(mut self, pages: usize) -> Self {
        self.max_pages = Some(pages);
        self
    }

    pub fn host_limits(mut self, limits: HostLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    #[must_use]
    pub fn stats(&self) -> &CrawlStats {
        &self.stats
    }

    pub async fn next(&mut self) -> Option<Result<CrawlPage>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }

    fn start(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let limits = match self.limits.take() {
            Some(limits) => Some(limits),
            None if self.session.host_limits().is_unlimited() => Some(polite_limits()),
            None => None,
        };
        if let Some(limits) = limits {
            self.session = self.session.with_host_limits(limits);
        }
        for seed in std::mem::take(&mut self.seeds) {
            match seed {
                Ok(url) => self.frontier.add(Queued {
                    url,
                    depth: 0,
                    hops: 0,
                    seed: true,
                }),
                Err(err) => self.ready.push_back(Err(err)),
            }
        }
    }

    fn budget_spent(&self) -> bool {
        self.max_pages
            .is_some_and(|max| self.stats.pages() as usize + self.pages_in_flight >= max)
    }

    fn read_sitemap(&mut self, url: Url) {
        let enough = self
            .max_pages
            .is_some_and(|max| self.frontier.known() >= max);
        if enough
            || self.sitemap_files.len() >= MAX_SITEMAP_FILES
            || !self.sitemap_files.insert(url.as_str().to_owned())
        {
            return;
        }
        let session = self.session.clone();
        self.tasks.push(Box::pin(async move {
            Event::Sitemap(sitemap::read(session, url).await)
        }));
    }

    fn fetch(&mut self, queued: Queued) {
        self.pages_in_flight += 1;
        self.stats.request();
        let session = self.session.clone();
        self.tasks.push(Box::pin(async move {
            let fetched = fetch(session, queued.url.clone()).await;
            Event::Page { queued, fetched }
        }));
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Robots { site, robots } => self.robots_read(site, *robots),
            Event::Sitemap(found) if found.index => {
                for url in found.locs.iter().filter_map(|loc| Url::parse(loc).ok()) {
                    self.read_sitemap(url);
                }
            }
            Event::Sitemap(found) => {
                for url in found.locs.iter().filter_map(|loc| Url::parse(loc).ok()) {
                    self.frontier.add(Queued {
                        url,
                        depth: 0,
                        hops: 0,
                        seed: false,
                    });
                }
            }
            Event::Page { queued, fetched } => {
                self.pages_in_flight -= 1;
                match fetched {
                    Err(err) => {
                        self.stats.error();
                        self.ready.push_back(Err(err));
                    }
                    Ok(Fetched::Redirect(_)) if queued.hops >= MAX_REDIRECTS => {
                        self.stats.skip(SkipReason::Redirects);
                    }
                    Ok(Fetched::Redirect(url)) => self.frontier.add(Queued {
                        url,
                        depth: queued.depth,
                        hops: queued.hops + 1,
                        seed: queued.seed,
                    }),
                    Ok(Fetched::Skip(reason)) => self.stats.skip(reason),
                    Ok(Fetched::Page(loaded)) => self.page(queued.depth, *loaded),
                }
            }
        }
    }
}

impl Stream for Crawl {
    type Item = Result<CrawlPage>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.start();
        loop {
            this.schedule();
            if let Some(item) = this.ready.pop_front() {
                return Poll::Ready(Some(item));
            }
            match this.tasks.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => this.handle(event),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl std::fmt::Debug for Crawl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crawl")
            .field("robots", &self.robots)
            .field("purpose", &self.purpose)
            .field("in_flight", &self.tasks.len())
            .field("stats", &self.stats)
            .finish()
    }
}
