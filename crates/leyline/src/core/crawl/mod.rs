mod checkpoint;
mod clock;
mod fetch;
mod frontier;
mod page;
mod sitemap;
mod sites;
mod visit;

use std::collections::{HashMap, VecDeque};
use std::future::{Future, poll_fn};
use std::pin::Pin;

use futures_util::stream::{FuturesUnordered, Stream};
use url::Url;

use fetch::{Fetched, fetch};
pub use frontier::Scope;
use frontier::{Frontier, Queued, page_key, same_site};
use page::CrawlStats;
pub use page::{Page, SkipReason};
use sitemap::SitemapRead;
use sites::{Site, SitemapFile, Sitemaps};
use visit::Dedup;

use crate::core::config::{HostGates, HostLimits, Origin};
use crate::core::into_url::IntoUrl;
use crate::core::retry::Retry;
use crate::core::robots::{ANY_AGENT, MAX_REDIRECTS, Purpose, Robots};
use crate::core::session::Session;
use crate::error::Result;

const NOFOLLOW: &str = "nofollow";
const PAGE_RETRIES: u32 = 2;
const MIN_PRINT_WORDS: usize = 20;
const MAX_IN_FLIGHT: usize = 16;
const POLITE_IN_FLIGHT: usize = 1;
const POLITE_PER_SECOND: f64 = 1.0;
const PAUSE_STATUSES: [u16; 2] = [429, 503];

type Task = Pin<Box<dyn Future<Output = Event> + Send>>;

enum Event {
    Robots {
        origin: Origin,
        url: Url,
        robots: Result<Box<Robots>>,
    },
    Page {
        origin: Origin,
        queued: Queued,
        fetched: Result<Fetched>,
    },
    Sitemap {
        file: SitemapFile,
        read: SitemapRead,
    },
}

/// Which site signals a crawl obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Respect {
    /// Obeys robots.txt rules and noindex directives.
    Robots,
    /// Obeys robots.txt and also skips pages that refuse AI training use.
    AiTraining,
}

#[doc = include_str!("../../../docs/crawl.md")]
#[must_use = "a crawl is lazy: nothing happens until `.next()` is awaited"]
pub struct Crawl {
    session: Session,
    limits: Option<HostLimits>,
    pace: HostGates,
    seeds: Vec<Result<Url>>,
    settings: Settings,
    started: bool,
    frontier: Frontier,
    sites: HashMap<Origin, Site>,
    busy: HashMap<Origin, usize>,
    sitemaps: Sitemaps,
    trusted: Option<Url>,
    page_retry: Retry,
    clock: clock::Clock,
    tasks: FuturesUnordered<Task>,
    pages_in_flight: usize,
    ready: VecDeque<Result<Page>>,
    dedup: Dedup,
    stats: CrawlStats,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Settings {
    respect: Option<Respect>,
    agent: String,
    max_body: Option<usize>,
    max_pages: Option<usize>,
}

fn polite_limits() -> HostLimits {
    HostLimits::new()
        .max_in_flight(POLITE_IN_FLIGHT)
        .per_second(POLITE_PER_SECOND)
        .pause_on(PAUSE_STATUSES)
}

fn page_retry(session: &Retry) -> Retry {
    match session.is_none() {
        true => Retry::new(PAGE_RETRIES),
        false => session.clone(),
    }
}

impl Session {
    /// Starts a lazy crawl from `seed`, resolved against the session's base URL, that respects robots.txt by default.
    pub fn crawl(&self, seed: impl IntoUrl) -> Crawl {
        Crawl::new(self, vec![seed.into_url_with_base(self.base_url())])
    }
}

impl Crawl {
    fn new(session: &Session, seeds: Vec<Result<Url>>) -> Self {
        Crawl {
            session: session.clone(),
            limits: None,
            pace: HostGates::new(HostLimits::new()),
            seeds,
            settings: Settings {
                respect: Some(Respect::Robots),
                agent: ANY_AGENT.to_owned(),
                max_body: None,
                max_pages: None,
            },
            started: false,
            frontier: Frontier::default(),
            sites: HashMap::new(),
            busy: HashMap::new(),
            sitemaps: Sitemaps::default(),
            trusted: None,
            page_retry: Retry::new(0),
            clock: clock::Clock::default(),
            tasks: FuturesUnordered::new(),
            pages_in_flight: 0,
            ready: VecDeque::new(),
            dedup: Dedup::default(),
            stats: CrawlStats::default(),
        }
    }
}

impl Crawl {
    /// Sets which site signals the crawl obeys; `None` ignores them all.
    pub fn respect(mut self, respect: impl Into<Option<Respect>>) -> Self {
        self.settings.respect = respect.into();
        self
    }

    /// Sets the robots.txt group and meta robots name the crawl obeys, falling back to `*`.
    pub fn agent(mut self, agent: impl Into<String>) -> Self {
        self.settings.agent = agent.into();
        self
    }

    /// Sets the largest page body the crawl reads, overriding the session's `max_body_size`.
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.settings.max_body = Some(bytes);
        self
    }

    /// Sets which URLs the crawl follows, `Scope::Host` by default.
    pub fn scope(mut self, scope: Scope) -> Self {
        self.frontier.scope = scope;
        self
    }

    /// Stops the crawl after it yields `pages` pages; unlimited by default.
    pub fn limit(mut self, pages: usize) -> Self {
        self.settings.max_pages = Some(pages);
        self
    }

    /// Replaces the session's host limits for this crawl.
    pub fn host_limits(mut self, limits: HostLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Returns how many page requests the crawl has sent.
    #[must_use]
    pub fn requests(&self) -> u64 {
        self.stats.requests()
    }

    /// Returns how many pages the crawl has yielded.
    #[must_use]
    pub fn pages(&self) -> u64 {
        self.stats.pages()
    }

    /// Returns how many fetches the crawl has failed.
    #[must_use]
    pub fn errors(&self) -> u64 {
        self.stats.errors()
    }

    /// Returns how many URLs the crawl has skipped for `reason`.
    #[must_use]
    pub fn skipped(&self, reason: SkipReason) -> u64 {
        self.stats.skipped(reason)
    }

    /// Returns the next crawled page or error, or `None` when the crawl is done.
    pub async fn next(&mut self) -> Option<Result<Page>> {
        poll_fn(move |cx| Pin::new(&mut *self).poll_next(cx)).await
    }

    fn purpose(&self) -> Option<Purpose> {
        match self.settings.respect? {
            Respect::Robots => None,
            Respect::AiTraining => Some(Purpose::AiTrain),
        }
    }

    fn start(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let pace = match self.limits.take() {
            Some(limits) => match limits.validate() {
                Err(error) => {
                    self.ready.push_back(Err(error));
                    self.seeds.clear();
                    return;
                }
                Ok(()) => HostGates::new(limits),
            },
            None => self
                .session
                .host_limits()
                .derived(|own| own.clone().filled_from(&polite_limits())),
        };
        let pace = match self.settings.respect {
            Some(_) => pace,
            None => pace.without_floors(),
        };
        self.session = self.session.with_extra_gates(pace.clone());
        self.page_retry = page_retry(self.session.default_retry());
        self.clock.stopped = Some(self.session.stopped());
        self.pace = pace;
        for seed in std::mem::take(&mut self.seeds) {
            match seed {
                Ok(url) => {
                    self.trusted.get_or_insert_with(|| url.clone());
                    self.frontier.add(Queued::seed(url));
                }
                Err(err) => self.ready.push_back(Err(err)),
            }
        }
    }

    fn limit_met(&self) -> bool {
        self.settings
            .max_pages
            .is_some_and(|max| self.stats.pages() as usize >= max)
    }

    fn budget_spent(&self) -> bool {
        self.settings.max_pages.is_some_and(|max| {
            self.stats.pages() as usize + self.pages_in_flight + self.dedup.holding() >= max
        })
    }

    fn fetch(&mut self, origin: Origin, queued: Queued) {
        self.pages_in_flight += 1;
        *self.busy.entry(origin.clone()).or_default() += 1;
        self.stats.request();
        let session = self.session.clone();
        let (retry, trusted) = (self.page_retry.clone(), self.trusted.clone());
        let (attempt, max_body) = (queued.attempts, self.settings.max_body);
        self.tasks.push(Box::pin(async move {
            let fetched = fetch(
                session,
                queued.url.clone(),
                retry,
                attempt,
                trusted,
                max_body,
            )
            .await;
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

    fn redirect(&mut self, queued: Queued, to: Url) {
        if queued.hops >= MAX_REDIRECTS {
            self.stats.skip(SkipReason::Redirects);
            return;
        }
        let from = page_key(&queued.url);
        let target = page_key(&to);
        if target == from || queued.chain.contains(&target) {
            self.frontier.redirected(&to);
        }
        let widen =
            queued.seed && same_site(&queued.url, &to) && self.frontier.in_scope(&queued.url);
        let seed = queued.seed && (queued.hops == 0 || widen || self.frontier.in_scope(&to));
        let seen = self.frontier.has_seen(&to);
        let mut chain = queued.chain;
        chain.push(from);
        let admitted = self.frontier.add_redirect(Queued {
            url: to,
            depth: queued.depth,
            hops: queued.hops + 1,
            seed,
            widen,
            attempts: 0,
            chain,
        });
        match (admitted, seen) {
            (false, true) => self.stats.skip(SkipReason::Duplicate),
            (false, false) => self.stats.skip(SkipReason::Scope),
            (true, _) => {}
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Robots {
                origin,
                url,
                robots: Ok(robots),
            } => self.robots_read(origin, url, *robots),
            Event::Robots {
                origin,
                robots: Err(error),
                ..
            } => {
                self.sites.remove(&origin);
                self.ready.push_back(Err(error));
            }
            Event::Sitemap { file, read } => self.sitemap_read(file, read),
            Event::Page {
                origin,
                queued,
                fetched,
            } => {
                self.release(&origin);
                let key = page_key(&queued.url);
                match fetched {
                    Err(err) => {
                        self.stats.error();
                        self.ready.push_back(Err(err));
                    }
                    Ok(Fetched::Redirect { to }) => {
                        let target = page_key(&to);
                        self.redirect(queued, to);
                        return self.forward(key, target);
                    }
                    Ok(Fetched::Later(wait)) => return self.later(queued, wait),
                    Ok(Fetched::Skip(reason)) => self.stats.skip(reason),
                    Ok(Fetched::Page(loaded)) => self.page(queued.depth, *loaded),
                }
                self.settle(key);
            }
        }
    }
}

impl std::fmt::Debug for Crawl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crawl")
            .field("respect", &self.settings.respect)
            .field("in_flight", &self.tasks.len())
            .field("stats", &self.stats)
            .finish()
    }
}
