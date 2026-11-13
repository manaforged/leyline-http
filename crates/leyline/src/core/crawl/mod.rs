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
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, Stream};
use url::Url;

use fetch::{Fetched, fetch};
pub use frontier::Scope;
use frontier::{Frontier, Queued, page_key, same_site};
use page::CrawlStats;
pub use page::{Page, SkipReason};
use sitemap::SitemapRead;
use sites::{
    Site, SitemapFile, Sitemaps, default_max_crawl_delay, default_max_sitemaps,
    default_robots_max_age,
};
use visit::Dedup;

use crate::core::Redirect;
use crate::core::config::{HostGates, HostLimits, Origin};
use crate::core::into_url::IntoUrl;
use crate::core::retry::Retry;
use crate::core::robots::{ANY_AGENT, Purpose, Robots};
use crate::core::session::Session;
use crate::error::Result;

const NOFOLLOW: &str = "nofollow";
const DEFAULT_MIN_DEDUP_WORDS: usize = 20;
const POLITE_IN_FLIGHT: usize = 1;
const POLITE_TOTAL_IN_FLIGHT: usize = 16;
const POLITE_PER_SECOND: f64 = 1.0;
const PAUSE_STATUSES: [u16; 2] = [429, 503];

fn default_dedup() -> bool {
    true
}

fn default_dedup_min_words() -> Option<usize> {
    Some(DEFAULT_MIN_DEDUP_WORDS)
}

pub(super) fn min_some(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

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
        origin: Origin,
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
    redirect_override: Option<Redirect>,
    redirect_policy: Redirect,
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
    #[serde(default = "default_max_sitemaps")]
    max_sitemaps: Option<usize>,
    #[serde(default = "default_max_crawl_delay")]
    max_crawl_delay: Option<Duration>,
    #[serde(default = "default_robots_max_age")]
    robots_max_age: Option<Duration>,
    #[serde(default = "default_dedup")]
    dedup: bool,
    #[serde(default = "default_dedup_min_words")]
    dedup_min_words: Option<usize>,
    #[serde(default)]
    redirect_hops: Option<Option<usize>>,
}

fn polite_limits() -> HostLimits {
    HostLimits::new()
        .max_in_flight(POLITE_IN_FLIGHT)
        .max_total_in_flight(POLITE_TOTAL_IN_FLIGHT)
        .per_second(POLITE_PER_SECOND)
        .pause_on(PAUSE_STATUSES)
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
            redirect_override: None,
            redirect_policy: session.redirect_policy().for_robots(),
            pace: HostGates::new(HostLimits::new()),
            seeds,
            settings: Settings {
                respect: Some(Respect::Robots),
                agent: ANY_AGENT.to_owned(),
                max_body: None,
                max_pages: None,
                max_sitemaps: default_max_sitemaps(),
                max_crawl_delay: default_max_crawl_delay(),
                robots_max_age: default_robots_max_age(),
                dedup: default_dedup(),
                dedup_min_words: default_dedup_min_words(),
                redirect_hops: None,
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

    /// Sets the largest body the crawl reads, overriding the session's `max_body_size`; `None` removes the cap.
    pub fn max_body_size(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.settings.max_body = Some(bytes.into().unwrap_or(crate::wire::NO_SIZE_CAP));
        self
    }

    /// Sets which URLs the crawl follows, `Scope::Host` by default.
    pub fn scope(mut self, scope: Scope) -> Self {
        self.frontier.scope = scope;
        self
    }

    /// Follows a discovered link, sitemap URL or redirect target only when `keep` returns true for it, on top of the scope and never for a seed; a checkpoint does not save the filter, so call this again after `Crawl::load`.
    pub fn filter<F>(mut self, keep: F) -> Self
    where
        F: Fn(&Url) -> bool + Send + Sync + 'static,
    {
        self.frontier.filter = Some(Arc::new(keep));
        self
    }

    /// Stops the crawl after it yields `pages` pages, or removes the cap when `None`.
    pub fn limit(mut self, pages: impl Into<Option<usize>>) -> Self {
        self.settings.max_pages = pages.into();
        self
    }

    /// Replaces the redirect policy for this crawl; 5 hops by default when neither crawl nor session sets one.
    pub fn redirect(mut self, policy: impl Into<Redirect>) -> Self {
        let policy = policy.into();
        self.settings.redirect_hops = Some(policy.hop_limit());
        self.redirect_override = Some(policy);
        self
    }

    /// Sets the furthest link depth from a seed the crawl follows, 64 by default, or unlimited when `None`.
    pub fn max_depth(mut self, depth: impl Into<Option<usize>>) -> Self {
        self.frontier.max_depth = depth.into();
        self
    }

    /// Sets the longest URL in bytes the crawl queues, 2048 by default, or unlimited when `None`.
    pub fn max_url_size(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.frontier.max_url_size = bytes.into();
        self
    }

    /// Sets the most sitemap files the crawl opens, 50 by default, or unlimited when `None`.
    pub fn max_sitemaps(mut self, files: impl Into<Option<usize>>) -> Self {
        self.settings.max_sitemaps = files.into();
        self
    }

    /// Caps the robots.txt `Crawl-delay` the crawl honors, 30 seconds by default, or uncapped when `None`.
    pub fn max_crawl_delay(mut self, delay: impl Into<Option<Duration>>) -> Self {
        self.settings.max_crawl_delay = delay.into();
        self
    }

    /// Sets how long a fetched robots.txt stays fresh before re-reading, 24 hours by default, or forever when `None`.
    pub fn robots_max_age(mut self, max_age: impl Into<Option<Duration>>) -> Self {
        self.settings.robots_max_age = max_age.into();
        self
    }

    /// Sets whether canonical and duplicate-text pages are skipped, `true` by default.
    pub fn dedup(mut self, enabled: bool) -> Self {
        self.settings.dedup = enabled;
        self
    }

    /// Sets the word-count floor for duplicate-text detection, 20 by default, or `None` to disable text matching.
    pub fn dedup_min_words(mut self, words: impl Into<Option<usize>>) -> Self {
        self.settings.dedup_min_words = words.into();
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

    pub(super) fn max_active(&self) -> usize {
        min_some(
            self.pace.max_total(),
            self.session.host_limits().max_total(),
        )
        .unwrap_or_else(|| self.session.max_connections())
        .max(1)
    }

    pub(super) fn origin_cap(&self, origin: &Origin) -> usize {
        min_some(
            self.pace.max_in_flight_for(origin),
            self.session.host_limits().max_in_flight_for(origin),
        )
        .unwrap_or_else(|| self.max_active())
        .max(1)
    }

    fn resolve_pace(&mut self) -> Option<HostGates> {
        let pace = match self.limits.take() {
            Some(limits) => match limits.validate() {
                Err(error) => {
                    self.ready.push_back(Err(error));
                    self.seeds.clear();
                    return None;
                }
                Ok(()) => HostGates::new(limits),
            },
            None => self
                .session
                .host_limits()
                .derived(|own| own.clone().filled_from(&polite_limits())),
        };
        Some(match self.settings.respect {
            Some(_) => pace,
            None => pace.without_floors(),
        })
    }

    fn resolve_redirect(&mut self) -> Redirect {
        if let Some(policy) = self.redirect_override.take() {
            return policy;
        }
        match self.settings.redirect_hops {
            Some(Some(0)) => Redirect::none(),
            Some(Some(max)) => Redirect::limit(max),
            Some(None) | None => self.session.redirect_policy().for_robots(),
        }
    }

    fn start(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let Some(pace) = self.resolve_pace() else {
            return;
        };
        let redirect = self.resolve_redirect();
        self.session =
            self.session
                .for_crawl(pace.clone(), self.settings.max_body, redirect.clone());
        self.redirect_policy = redirect;
        self.page_retry = self.session.default_retry().for_crawl_page();
        self.clock.stopped = Some(self.session.stopped());
        self.pace = pace;
        for seed in std::mem::take(&mut self.seeds) {
            match seed {
                Ok(url) => {
                    self.trusted.get_or_insert_with(|| url.clone());
                    self.frontier.add(Queued::seed(url), &mut self.stats);
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
        let attempt = queued.attempts;
        self.tasks.push(Box::pin(async move {
            let fetched = fetch(session, queued.url.clone(), retry, attempt, trusted).await;
            Event::Page {
                origin,
                queued,
                fetched,
            }
        }));
    }

    fn release_origin(&mut self, origin: &Origin) {
        if let Some(busy) = self.busy.get_mut(origin) {
            *busy -= 1;
            if *busy == 0 {
                self.busy.remove(origin);
            }
        }
    }

    fn follow_redirect(&mut self, mut queued: Queued, status: u16, location: String, to: Url) {
        let hop = self
            .redirect_policy
            .check_hops(queued.hops + 1)
            .ok()
            .and_then(|()| {
                self.redirect_policy
                    .approved_hop(status, &queued.url, &location, &queued.previous)
            });
        let Some(hop) = hop else {
            self.stats.skip(SkipReason::Redirects);
            return;
        };
        let from = page_key(&queued.url);
        let target = page_key(&to);
        if target == from || queued.chain.contains(&target) {
            self.frontier.redirected(&to);
        }
        let widen =
            queued.seed && same_site(&queued.url, &to) && self.frontier.in_scope(&queued.url);
        let seed = queued.seed && (queued.hops == 0 || widen || self.frontier.in_scope(&to));
        queued.chain.push(from);
        queued.previous.push(hop);
        self.frontier.add_redirect(
            Queued {
                url: to,
                depth: queued.depth,
                hops: queued.hops + 1,
                seed,
                widen,
                attempts: 0,
                chain: queued.chain,
                previous: queued.previous,
            },
            &mut self.stats,
        );
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
            Event::Sitemap { origin, file, read } => {
                self.release_origin(&origin);
                self.sitemap_read(file, read);
            }
            Event::Page {
                origin,
                queued,
                fetched,
            } => {
                self.pages_in_flight -= 1;
                self.release_origin(&origin);
                let key = page_key(&queued.url);
                match fetched {
                    Err(err) => {
                        self.stats.error();
                        self.ready.push_back(Err(err));
                    }
                    Ok(Fetched::Redirect {
                        status,
                        location,
                        to,
                    }) => {
                        let target = page_key(&to);
                        self.follow_redirect(queued, status, location, to);
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
