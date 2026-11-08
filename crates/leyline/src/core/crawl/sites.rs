use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::frontier::{Queued, page_key, same_site};
use super::sitemap::SitemapRead;
use super::{Crawl, Event, MAX_IN_FLIGHT, SkipReason, Task, sitemap};
use crate::core::config::Origin;
use crate::core::robots::{Robots, RobotsSource, crawl_retry, fallback};
use crate::net::normalize_host;
use crate::util::redact;

const ROBOTS_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);
const MAX_SITEMAP_FILES: usize = 50;
const MAX_SITEMAP_TASKS: usize = 2;

pub(super) enum Site {
    Pending,
    Ready(Robots, Instant),
}

fn fresh(read: &Instant) -> bool {
    read.elapsed() < ROBOTS_MAX_AGE
}

enum SitemapGate {
    Open,
    Wait,
    Drop,
    Robots,
}

pub(super) struct SitemapFile {
    pub(super) url: Url,
    pub(super) attempts: u32,
    pub(super) not_before: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Sitemaps {
    pub(super) files: HashSet<String>,
    pub(super) queue: VecDeque<SitemapFile>,
    in_flight: usize,
}

impl Crawl {
    fn verdict(&self, robots: &Robots, url: &Url) -> Option<SkipReason> {
        if self.settings.respect.is_some() && !robots.allows(url) {
            return Some(SkipReason::Robots);
        }
        let refused = self
            .purpose()
            .is_some_and(|purpose| !robots.allows_use(url, purpose));
        refused.then_some(SkipReason::Signal)
    }

    pub(super) fn schedule(&mut self) {
        self.promote_due();
        while self.active() < MAX_IN_FLIGHT {
            if self.open_sitemap() {
                continue;
            }
            if self.budget_spent() {
                break;
            }
            let Some(queued) = self.pop_ready() else {
                break;
            };
            self.start_page(queued);
        }
        self.plan_alarm();
    }

    fn pop_ready(&mut self) -> Option<Queued> {
        let (sites, busy, pace) = (&self.sites, &self.busy, &self.pace);
        let own = self.session.host_limits();
        let obey = self.settings.respect.is_some();
        self.frontier.pop_where(|origin| {
            let cap = pace.max_in_flight_for(origin).unwrap_or(MAX_IN_FLIGHT);
            !(obey && matches!(sites.get(origin), Some(Site::Pending)))
                && busy.get(origin).copied().unwrap_or(0) < cap
                && pace.ready(origin)
                && own.ready(origin)
        })
    }

    fn known_verdict(&self, origin: &Origin, url: &Url) -> Option<Option<SkipReason>> {
        match self.sites.get(origin) {
            Some(Site::Ready(robots, read)) if fresh(read) => Some(self.verdict(robots, url)),
            _ => None,
        }
    }

    fn start_page(&mut self, queued: Queued) {
        let Some(origin) = Origin::of(&queued.url) else {
            return;
        };
        let upgraded = self.session.upgraded(&queued.url);
        if upgraded != queued.url {
            self.forward(page_key(&queued.url), page_key(&upgraded));
            self.frontier.rekey(queued, upgraded);
            return;
        }
        match self.known_verdict(&origin, &queued.url) {
            None if self.settings.respect.is_none() => {
                let skipped = Robots::empty(RobotsSource::Unavailable);
                self.robots_read(origin.clone(), queued.url.clone(), skipped);
                self.fetch(origin, queued);
            }
            None => {
                self.read_robots(origin, queued.url.clone());
                self.frontier.push_front(queued);
            }
            Some(Some(reason)) => {
                self.stats.skip(reason);
                self.settle(page_key(&queued.url));
            }
            Some(None) => self.fetch(origin, queued),
        }
    }

    fn active(&self) -> usize {
        self.tasks.len()
    }

    fn read_robots(&mut self, origin: Origin, url: Url) {
        self.tasks.push(self.robots_task(origin.clone(), url));
        self.sites.insert(origin, Site::Pending);
    }

    fn robots_task(&self, origin: Origin, url: Url) -> Task {
        let session = self.session.clone();
        let trusted = self.trusted.clone();
        let agent = self.settings.agent.clone();
        Box::pin(async move {
            let robots = session
                .robots_with(url.as_str(), &agent, crawl_retry(), trusted)
                .await
                .or_else(fallback);
            Event::Robots {
                origin,
                url,
                robots: robots.map(Box::new),
            }
        })
    }

    pub(super) fn robots_read(&mut self, origin: Origin, url: Url, robots: Robots) {
        if self.frontier.in_scope(&url) {
            for sitemap in sitemap::listed(&url, &robots, self.settings.respect.is_some()) {
                if same_site(&url, &sitemap) {
                    self.queue_sitemap(sitemap, None);
                }
            }
        }
        if self.settings.respect.is_some() {
            let gap = robots.crawl_delay().map(|gap| gap.min(MAX_CRAWL_DELAY));
            self.pace.set_floor(&url, gap);
            self.pace.set_floor(&self.session.upgraded(&url), gap);
        }
        self.sites
            .insert(origin, Site::Ready(robots, Instant::now()));
    }

    pub(super) fn queue_sitemap(&mut self, url: Url, parent: Option<Url>) {
        let same_site = parent.as_ref().is_none_or(|parent| {
            parent.host_str().map(normalize_host) == url.host_str().map(normalize_host)
        });
        if same_site
            && self.sitemaps.files.len() < MAX_SITEMAP_FILES
            && self.sitemaps.files.insert(url.as_str().to_owned())
        {
            self.sitemaps.queue.push_back(SitemapFile {
                url,
                attempts: 0,
                not_before: None,
            });
        }
    }

    fn sitemaps_enough(&self) -> bool {
        self.settings
            .max_pages
            .is_some_and(|max| self.frontier.known() >= max)
    }

    fn open_sitemap(&mut self) -> bool {
        if self.sitemaps.in_flight >= MAX_SITEMAP_TASKS {
            return false;
        }
        if self.sitemaps_enough() {
            self.sitemaps.queue.clear();
            return false;
        }
        let mut waiting = VecDeque::new();
        let mut opened = false;
        while let Some(file) = self.sitemaps.queue.pop_front() {
            let Some(origin) = Origin::of(&file.url) else {
                continue;
            };
            match self.sitemap_gate(&origin, &file) {
                SitemapGate::Wait => waiting.push_back(file),
                SitemapGate::Robots => {
                    self.read_robots(origin, file.url.clone());
                    waiting.push_back(file);
                    opened = true;
                }
                SitemapGate::Drop => self.stats.skip(SkipReason::Robots),
                SitemapGate::Open => {
                    self.start_sitemap(file);
                    opened = true;
                    break;
                }
            }
        }
        waiting.append(&mut self.sitemaps.queue);
        self.sitemaps.queue = waiting;
        opened
    }

    fn sitemap_gate(&self, origin: &Origin, file: &SitemapFile) -> SitemapGate {
        let due = file.not_before.is_none_or(|at| at <= Instant::now());
        if !due || self.active() >= MAX_IN_FLIGHT {
            return SitemapGate::Wait;
        }
        if !(self.pace.ready(origin) && self.session.host_limits().ready(origin)) {
            return SitemapGate::Wait;
        }
        if self.settings.respect.is_none() {
            return SitemapGate::Open;
        }
        match self.sites.get(origin) {
            Some(Site::Ready(_, read)) if !fresh(read) => SitemapGate::Robots,
            Some(Site::Ready(robots, _)) if !robots.allows(&file.url) => SitemapGate::Drop,
            Some(Site::Ready(..)) => SitemapGate::Open,
            Some(Site::Pending) => SitemapGate::Wait,
            None => SitemapGate::Robots,
        }
    }

    fn start_sitemap(&mut self, file: SitemapFile) {
        self.sitemaps.in_flight += 1;
        let session = self.session.clone();
        let (retry, trusted) = (self.page_retry.clone(), self.trusted.clone());
        self.tasks.push(Box::pin(async move {
            let read =
                sitemap::read(session, file.url.clone(), retry, file.attempts, trusted).await;
            Event::Sitemap { file, read }
        }));
    }

    pub(super) fn sitemap_read(&mut self, mut file: SitemapFile, read: SitemapRead) {
        self.sitemaps.in_flight -= 1;
        let found = match read {
            SitemapRead::Found(found) => found,
            SitemapRead::Moved(to) => return self.queue_sitemap(to, Some(file.url)),
            SitemapRead::Later(wait) => {
                let Some(at) = Instant::now().checked_add(wait) else {
                    return self.stats.error();
                };
                file.attempts += 1;
                file.not_before = Some(at);
                self.sitemaps.queue.push_back(file);
                return self.set_alarm(at);
            }
            SitemapRead::Failed(cause) => {
                tracing::debug!(
                    target: "leyline::crawl",
                    url = %redact(file.url.as_str()),
                    %cause,
                    "sitemap read failed"
                );
                return self.stats.error();
            }
        };
        let url = file.url;
        for loc in found.locs.iter().filter_map(|loc| Url::parse(loc).ok()) {
            if found.index {
                self.queue_sitemap(loc, Some(url.clone()));
            } else {
                self.frontier.add(Queued::link(loc, 0));
            }
        }
    }
}
