mod fetch;
mod frontier;
mod page;
mod sitemap;
mod sites;
mod visit;

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::{FuturesUnordered, Stream, StreamExt};
use url::Url;

use fetch::{Fetched, fetch};
pub use frontier::Scope;
use frontier::{Frontier, Queued, page_key, same_site};
pub use page::{CrawlPage, CrawlStats, SkipReason};
use sites::{Site, SitemapFile};

use crate::core::config::{HostLimits, Origin};
use crate::core::error::{Error, Kind, Result};
use crate::core::into_url::IntoUrl;
use crate::core::robots::{Purpose, Robots};
use crate::core::session::Session;
use crate::html::Sitemap;

const ANY_AGENT: &str = "*";
const NOFOLLOW: &str = "nofollow";
const MAX_REDIRECTS: usize = 5;
const MAX_ATTEMPTS: u8 = 3;
const MAX_IN_FLIGHT: usize = 16;
const POLITE_IN_FLIGHT: usize = 1;
const POLITE_PER_SECOND: f64 = 1.0;
const PAUSE_STATUSES: [u16; 2] = [429, 503];

type Task = Pin<Box<dyn Future<Output = Event> + Send>>;

enum Event {
    Robots {
        origin: Origin,
        url: Url,
        robots: Box<Robots>,
    },
    Page {
        origin: Origin,
        queued: Queued,
        fetched: Result<Fetched>,
    },
    Sitemap {
        url: Url,
        found: Sitemap,
    },
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
    pace: HostLimits,
    seeds: Vec<Result<Url>>,
    respect: Option<Respect>,
    max_pages: Option<usize>,
    started: bool,
    frontier: Frontier,
    sites: HashMap<Origin, Site>,
    busy: HashMap<Origin, usize>,
    sitemap_files: HashSet<String>,
    sitemap_queue: VecDeque<SitemapFile>,
    sitemaps_in_flight: usize,
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

fn inherited(session: &Session) -> HostLimits {
    session.host_limits().clone().filled_from(&polite_limits())
}

impl Session {
    pub fn crawl(&self, seed: impl IntoUrl) -> Crawl {
        Crawl {
            session: self.clone(),
            limits: None,
            pace: HostLimits::new(),
            seeds: vec![seed.into_url_with_base(self.base_url())],
            respect: None,
            max_pages: None,
            started: false,
            frontier: Frontier::default(),
            sites: HashMap::new(),
            busy: HashMap::new(),
            sitemap_files: HashSet::new(),
            sitemap_queue: VecDeque::new(),
            sitemaps_in_flight: 0,
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
        self.respect = Some(respect);
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

    fn purpose(&self) -> Option<Purpose> {
        match self.respect? {
            Respect::Robots => None,
            Respect::AiTraining => Some(Purpose::AiTrain),
        }
    }

    fn start(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let pace = match self.limits.take() {
            Some(limits) => match limits.config_error() {
                Some(message) => {
                    let error = Error::new(Kind::Config).with_message(message);
                    self.ready.push_back(Err(error));
                    self.seeds.clear();
                    return;
                }
                None => limits.armed(),
            },
            None => self.session.crawl_limits(|| inherited(&self.session)),
        };
        self.session = self.session.with_host_limits(pace.clone());
        self.pace = pace;
        for seed in std::mem::take(&mut self.seeds) {
            match seed {
                Ok(url) => {
                    self.frontier.add(Queued::seed(url));
                }
                Err(err) => self.ready.push_back(Err(err)),
            }
        }
    }

    fn budget_spent(&self) -> bool {
        self.max_pages
            .is_some_and(|max| self.stats.pages() as usize + self.pages_in_flight >= max)
    }

    fn fetch(&mut self, origin: Origin, queued: Queued) {
        self.pages_in_flight += 1;
        *self.busy.entry(origin.clone()).or_default() += 1;
        self.stats.request();
        let session = self.session.clone();
        self.tasks.push(Box::pin(async move {
            let fetched = fetch(session, queued.url.clone()).await;
            Event::Page {
                origin,
                queued,
                fetched,
            }
        }));
    }

    fn release(&mut self, origin: &Origin) {
        self.pages_in_flight -= 1;
        if let Some(busy) = self.busy.get_mut(origin) {
            *busy -= 1;
            if *busy == 0 {
                self.busy.remove(origin);
            }
        }
    }

    fn redirect(&mut self, queued: Queued, to: Url, permanent: bool) {
        if queued.hops >= MAX_REDIRECTS {
            self.stats.skip(SkipReason::Redirects);
            return;
        }
        let from = page_key(&queued.url);
        let target = page_key(&to);
        if target == from || queued.chain.contains(&target) {
            self.frontier.redirected(&to);
        }
        let widen = queued.seed && (permanent || same_site(&queued.url, &to));
        let seen = self.frontier.has_seen(&to);
        let mut chain = queued.chain;
        chain.push(from);
        let admitted = self.frontier.add_redirect(Queued {
            url: to,
            depth: queued.depth,
            hops: queued.hops + 1,
            seed: queued.seed,
            widen,
            attempts: 0,
            chain,
        });
        if seen && !admitted {
            self.stats.skip(SkipReason::Duplicate);
        }
    }

    fn later(&mut self, mut queued: Queued) {
        if queued.attempts + 1 >= MAX_ATTEMPTS {
            self.stats.skip(SkipReason::Status);
            return;
        }
        queued.attempts += 1;
        self.frontier.push_back(queued);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Robots {
                origin,
                url,
                robots,
            } => self.robots_read(origin, url, *robots),
            Event::Sitemap { url, found } => self.sitemap_read(url, found.index, found.locs),
            Event::Page {
                origin,
                queued,
                fetched,
            } => {
                self.release(&origin);
                match fetched {
                    Err(err) => {
                        self.stats.error();
                        self.ready.push_back(Err(err));
                    }
                    Ok(Fetched::Redirect { to, permanent }) => self.redirect(queued, to, permanent),
                    Ok(Fetched::Later) => self.later(queued),
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
            .field("respect", &self.respect)
            .field("in_flight", &self.tasks.len())
            .field("stats", &self.stats)
            .finish()
    }
}
