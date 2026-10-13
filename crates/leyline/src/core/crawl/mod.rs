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
use sitemap::SitemapRead;
use sites::{Site, SitemapFile};
use visit::Held;

use crate::core::config::{HostLimits, Origin};
use crate::core::error::{Error, Kind, Result};
use crate::core::into_url::IntoUrl;
use crate::core::retry::{RetryPolicy, RetryTrigger};
use crate::core::robots::{Purpose, Robots};
use crate::core::session::Session;

const ANY_AGENT: &str = "*";
const NOFOLLOW: &str = "nofollow";
const MAX_REDIRECTS: usize = 5;
const PAGE_RETRIES: u32 = 2;
const PAGE_RETRY_ON: [RetryTrigger; 4] = [
    RetryTrigger::ConnectionError,
    RetryTrigger::Timeout,
    RetryTrigger::Status(502),
    RetryTrigger::Status(504),
];
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
        robots: Box<Robots>,
    },
    Page {
        origin: Origin,
        queued: Queued,
        fetched: Result<Fetched>,
    },
    Sitemap {
        url: Url,
        read: SitemapRead,
    },
    Wake,
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
    trusted: Option<Url>,
    plain: Session,
    page_retry: RetryPolicy,
    waking: Option<tokio::time::Instant>,
    holding: usize,
    tasks: FuturesUnordered<Task>,
    pages_in_flight: usize,
    ready: VecDeque<Result<CrawlPage>>,
    kept: HashSet<String>,
    texts: HashSet<u64>,
    held: HashMap<String, Vec<Held>>,
    waiting_on: HashMap<String, String>,
    settled: HashSet<String>,
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
            respect: Some(Respect::Robots),
            max_pages: None,
            started: false,
            frontier: Frontier::default(),
            sites: HashMap::new(),
            busy: HashMap::new(),
            sitemap_files: HashSet::new(),
            sitemap_queue: VecDeque::new(),
            sitemaps_in_flight: 0,
            trusted: None,
            plain: self.clone(),
            page_retry: RetryPolicy::none(),
            waking: None,
            holding: 0,
            tasks: FuturesUnordered::new(),
            pages_in_flight: 0,
            ready: VecDeque::new(),
            kept: HashSet::new(),
            texts: HashSet::new(),
            held: HashMap::new(),
            waiting_on: HashMap::new(),
            settled: HashSet::new(),
            stats: CrawlStats::default(),
        }
    }
}

impl Crawl {
    pub fn respect(mut self, respect: Respect) -> Self {
        self.respect = Some(respect);
        self
    }

    pub fn ignore_robots(mut self) -> Self {
        self.respect = None;
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
        let pace = match self.respect {
            Some(_) => pace,
            None => pace.without_floors(),
        };
        self.plain = self.session.clone();
        self.session = self.session.with_crawl_pace(pace.clone());
        self.page_retry = self
            .session
            .default_retry()
            .clone()
            .or(RetryPolicy::transient()
                .retry_on(PAGE_RETRY_ON)
                .max_retries(PAGE_RETRIES));
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
        self.max_pages
            .is_some_and(|max| self.stats.pages() as usize >= max)
    }

    fn budget_spent(&self) -> bool {
        self.max_pages.is_some_and(|max| {
            self.stats.pages() as usize + self.pages_in_flight + self.holding >= max
        })
    }

    fn fetch(&mut self, origin: Origin, queued: Queued) {
        self.pages_in_flight += 1;
        *self.busy.entry(origin.clone()).or_default() += 1;
        self.stats.request();
        let session = self.session.clone();
        let (retry, trusted) = (self.page_retry.clone(), self.trusted.clone());
        self.tasks.push(Box::pin(async move {
            let fetched = fetch(session, queued.url.clone(), retry, trusted).await;
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

    fn later(&mut self, mut queued: Queued) {
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
            Event::Sitemap { url, read } => self.sitemap_read(url, read),
            Event::Wake => self.waking = None,
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
                    Ok(Fetched::Later) if queued.attempts < PAGE_RETRIES as u8 => {
                        return self.later(queued);
                    }
                    Ok(Fetched::Later) => self.stats.skip(SkipReason::Status),
                    Ok(Fetched::Skip(reason)) => self.stats.skip(reason),
                    Ok(Fetched::Page(loaded)) => self.page(queued.depth, *loaded),
                }
                self.settle(key);
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
            if this.limit_met() && this.pages_in_flight == 0 {
                this.tasks = FuturesUnordered::new();
                return Poll::Ready(None);
            }
            match this.tasks.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => this.handle(event),
                Poll::Ready(None) if this.release_held() => {}
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
