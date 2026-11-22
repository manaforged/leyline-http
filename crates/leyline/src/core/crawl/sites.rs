use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::frontier::{Queued, Scope, page_key, same_site};
use super::sitemap::SitemapRead;
use super::{Crawl, Event, SkipReason, Task, min_some, sitemap};
use crate::core::config::Origin;
use crate::core::robots::{Robots, RobotsSource, fallback};
use crate::net::normalize_host;
use crate::util::redact;

const DEFAULT_ROBOTS_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_MAX_SITEMAPS: usize = 50;
const SITEMAP_SHARE_DIVISOR: usize = 8;

pub(super) fn default_robots_max_age() -> Option<Duration> {
    Some(DEFAULT_ROBOTS_MAX_AGE)
}

pub(super) fn default_max_crawl_delay() -> Option<Duration> {
    Some(DEFAULT_MAX_CRAWL_DELAY)
}

pub(super) fn default_max_sitemaps() -> Option<usize> {
    Some(DEFAULT_MAX_SITEMAPS)
}

pub(super) enum Site {
    Pending,
    Ready {
        robots: Robots,
        read: Instant,
        used: bool,
    },
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
    pub(super) hops: usize,
    pub(super) previous: Vec<Url>,
    pub(super) not_before: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Sitemaps {
    pub(super) files: HashSet<String>,
    pub(super) queue: VecDeque<SitemapFile>,
    in_flight: usize,
}

impl Sitemaps {
    pub(super) fn clear_in_flight(&mut self) {
        self.in_flight = 0;
    }
}

impl Crawl {
    fn robots_fresh(&self, read: Instant, used: bool) -> bool {
        !used
            || self
                .settings
                .robots_max_age
                .is_none_or(|max| read.elapsed() < max)
    }

    fn origin_ready(&self, origin: &Origin) -> bool {
        self.busy.get(origin).copied().unwrap_or(0) < self.origin_cap(origin)
            && self.pace.ready(origin)
            && self.session.host_limits().ready(origin)
    }

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
        while self.active() < self.max_active() {
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
        let fallback = self.max_active();
        let obey = self.settings.respect.is_some();
        self.frontier.pop_where(|origin| {
            let cap = min_some(
                pace.max_in_flight_for(origin),
                own.max_in_flight_for(origin),
            )
            .unwrap_or(fallback)
            .max(1);
            !(obey && matches!(sites.get(origin), Some(Site::Pending)))
                && busy.get(origin).copied().unwrap_or(0) < cap
                && pace.ready(origin)
                && own.ready(origin)
        })
    }

    fn take_verdict(&mut self, origin: &Origin, url: &Url) -> Option<Option<SkipReason>> {
        let verdict = match self.sites.get(origin) {
            Some(Site::Ready { robots, read, used }) if self.robots_fresh(*read, *used) => {
                self.verdict(robots, url)
            }
            _ => return None,
        };
        if let Some(Site::Ready { used, .. }) = self.sites.get_mut(origin) {
            *used = true;
        }
        Some(verdict)
    }

    fn start_page(&mut self, queued: Queued) {
        let Some(origin) = Origin::of(&queued.url) else {
            return;
        };
        let upgraded = self.session.upgraded(&queued.url);
        if upgraded != queued.url {
            self.forward(page_key(&queued.url), page_key(&upgraded));
            self.frontier.rekey(queued, upgraded, &mut self.stats);
            return;
        }
        match self.take_verdict(&origin, &queued.url) {
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
        let retry = session.default_retry().for_crawl_robots();
        Box::pin(async move {
            let robots = session
                .robots_with(url.as_str(), &agent, retry, trusted)
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
            let any_scope = self.frontier.scope == Scope::Any;
            for sitemap in sitemap::listed(&url, &robots, self.settings.respect.is_some()) {
                if any_scope || same_site(&url, &sitemap) {
                    self.queue_sitemap(sitemap, None);
                }
            }
        }
        if self.settings.respect.is_some() {
            let max_delay = self.settings.max_crawl_delay;
            let gap = robots
                .crawl_delay()
                .map(|gap| max_delay.map_or(gap, |max| gap.min(max)))
                .filter(|gap| !gap.is_zero());
            self.pace.set_floor(&url, gap);
            self.pace.set_floor(&self.session.upgraded(&url), gap);
        }
        self.sites.insert(
            origin,
            Site::Ready {
                robots,
                read: Instant::now(),
                used: false,
            },
        );
    }

    fn admits_sitemap(&mut self, url: &Url, parent: Option<&Url>) -> bool {
        let allowed_host = self.frontier.scope == Scope::Any
            || parent.is_none_or(|parent| {
                parent.host_str().map(normalize_host) == url.host_str().map(normalize_host)
            });
        let under_cap = self
            .settings
            .max_sitemaps
            .is_none_or(|max| self.sitemaps.files.len() < max);
        allowed_host && under_cap && self.sitemaps.files.insert(url.as_str().to_owned())
    }

    pub(super) fn queue_sitemap(&mut self, url: Url, parent: Option<Url>) {
        if self.admits_sitemap(&url, parent.as_ref()) {
            self.sitemaps.queue.push_back(SitemapFile {
                url,
                attempts: 0,
                hops: 0,
                previous: Vec::new(),
                not_before: None,
            });
        }
    }

    fn redirect_sitemap(&mut self, mut file: SitemapFile, status: u16, location: String, to: Url) {
        let hop = self
            .redirect_policy
            .check_hops(file.hops + 1)
            .ok()
            .and_then(|()| {
                self.redirect_policy
                    .approved_hop(status, &file.url, &location, &file.previous)
            });
        let Some(hop) = hop else {
            self.stats.skip(SkipReason::Redirects);
            return;
        };
        if self.admits_sitemap(&to, Some(&file.url)) {
            file.previous.push(hop);
            self.sitemaps.queue.push_back(SitemapFile {
                url: to,
                attempts: 0,
                hops: file.hops + 1,
                previous: file.previous,
                not_before: None,
            });
        }
    }

    fn sitemaps_enough(&self) -> bool {
        self.settings
            .max_pages
            .is_some_and(|max| self.frontier.known() >= max)
    }

    fn max_sitemap_tasks(&self) -> usize {
        if self.frontier.is_empty() {
            self.max_active()
        } else {
            (self.max_active() / SITEMAP_SHARE_DIVISOR).max(1)
        }
    }

    fn open_sitemap(&mut self) -> bool {
        if self.sitemaps.in_flight >= self.max_sitemap_tasks() {
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
                    self.start_sitemap(origin, file);
                    opened = true;
                    break;
                }
            }
        }
        waiting.append(&mut self.sitemaps.queue);
        self.sitemaps.queue = waiting;
        opened
    }

    fn sitemap_gate(&mut self, origin: &Origin, file: &SitemapFile) -> SitemapGate {
        let due = file.not_before.is_none_or(|at| at <= Instant::now());
        if !due || self.active() >= self.max_active() || !self.origin_ready(origin) {
            return SitemapGate::Wait;
        }
        if self.settings.respect.is_none() {
            return SitemapGate::Open;
        }
        let gate = match self.sites.get(origin) {
            Some(Site::Ready { read, used, .. }) if !self.robots_fresh(*read, *used) => {
                return SitemapGate::Robots;
            }
            Some(Site::Ready { robots, .. }) if !robots.allows(&file.url) => SitemapGate::Drop,
            Some(Site::Ready { .. }) => SitemapGate::Open,
            Some(Site::Pending) => return SitemapGate::Wait,
            None => return SitemapGate::Robots,
        };
        if let Some(Site::Ready { used, .. }) = self.sites.get_mut(origin) {
            *used = true;
        }
        gate
    }

    fn start_sitemap(&mut self, origin: Origin, file: SitemapFile) {
        self.sitemaps.in_flight += 1;
        *self.busy.entry(origin.clone()).or_default() += 1;
        let session = self.session.clone();
        let (retry, trusted) = (self.page_retry.clone(), self.trusted.clone());
        self.tasks.push(Box::pin(async move {
            let read =
                sitemap::read(session, file.url.clone(), retry, file.attempts, trusted).await;
            Event::Sitemap { origin, file, read }
        }));
    }

    pub(super) fn sitemap_read(&mut self, mut file: SitemapFile, read: SitemapRead) {
        self.sitemaps.in_flight -= 1;
        let found = match read {
            SitemapRead::Found(found) => found,
            SitemapRead::Moved {
                status,
                location,
                to,
            } => return self.redirect_sitemap(file, status, location, to),
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
                self.frontier.add(Queued::link(loc, 0), &mut self.stats);
            }
        }
    }
}
