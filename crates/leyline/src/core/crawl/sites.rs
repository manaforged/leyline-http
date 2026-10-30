use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::frontier::{Queued, page_key, same_site};
use super::sitemap::SitemapRead;
use super::{Crawl, Event, MAX_IN_FLIGHT, SkipReason, Task, sitemap};
use crate::core::config::{Origin, normalize_host};
use crate::core::retry::Retry;
use crate::core::robots::{Robots, RobotsSource};

const ROBOTS_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);
const MAX_SITEMAP_FILES: usize = 50;
const MAX_SITEMAP_TASKS: usize = 2;
const ROBOTS_RETRIES: u32 = 4;
const ROBOTS_BACKOFF: Duration = Duration::from_secs(1);
const ROBOTS_MAX_BACKOFF: Duration = Duration::from_secs(8);

pub(super) enum Site {
    Pending,
    Ready(Robots, Instant),
}

pub(super) struct SitemapFile {
    pub(super) url: Url,
    pub(super) attempts: u32,
    pub(super) not_before: Option<Instant>,
}

impl Crawl {
    fn verdict(&self, robots: &Robots, url: &Url) -> Option<SkipReason> {
        if self.respect.is_some() && !robots.allows(url) {
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
            let (sites, busy, pace) = (&self.sites, &self.busy, &self.pace);
            let own = self.session.host_limits();
            let obey = self.respect.is_some();
            let Some(queued) = self.frontier.pop_where(|origin| {
                let cap = pace.max_in_flight_for(origin).unwrap_or(MAX_IN_FLIGHT);
                !(obey && matches!(sites.get(origin), Some(Site::Pending)))
                    && busy.get(origin).copied().unwrap_or(0) < cap
                    && pace.ready(origin)
                    && own.ready(origin)
            }) else {
                break;
            };
            let Some(origin) = Origin::of(&queued.url) else {
                continue;
            };
            let upgraded = self.session.upgraded(&queued.url);
            if upgraded != queued.url {
                self.forward(page_key(&queued.url), page_key(&upgraded));
                self.frontier.rekey(queued, upgraded);
                continue;
            }
            let known = match self.sites.get(&origin) {
                Some(Site::Ready(robots, read)) if read.elapsed() < ROBOTS_MAX_AGE => {
                    Some(self.verdict(robots, &queued.url))
                }
                _ => None,
            };
            match known {
                None if self.respect.is_none() => {
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
        self.plan_alarm();
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
        let agent = self.agent.clone();
        let retry = Retry::new(ROBOTS_RETRIES)
            .when(|attempt| {
                attempt.is_transient() || attempt.status().is_some_and(|s| s.is_server_error())
            })
            .backoff(ROBOTS_BACKOFF, ROBOTS_MAX_BACKOFF);
        Box::pin(async move {
            let robots = match session
                .robots_with(url.as_str(), &agent, retry, trusted)
                .await
            {
                Ok(robots) => robots,
                Err(_) => Robots::empty(RobotsSource::Unreachable),
            };
            Event::Robots {
                origin,
                url,
                robots: Box::new(robots),
            }
        })
    }

    pub(super) fn robots_read(&mut self, origin: Origin, url: Url, robots: Robots) {
        if self.frontier.in_scope(&url) {
            for sitemap in sitemap::listed(&url, &robots, self.respect.is_some()) {
                if same_site(&url, &sitemap) {
                    self.queue_sitemap(sitemap, None);
                }
            }
        }
        if self.respect.is_some() {
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
            && self.sitemap_files.len() < MAX_SITEMAP_FILES
            && self.sitemap_files.insert(url.as_str().to_owned())
        {
            self.sitemap_queue.push_back(SitemapFile {
                url,
                attempts: 0,
                not_before: None,
            });
        }
    }

    fn sitemaps_enough(&self) -> bool {
        self.max_pages
            .is_some_and(|max| self.frontier.known() >= max)
    }

    fn open_sitemap(&mut self) -> bool {
        if self.sitemaps_in_flight >= MAX_SITEMAP_TASKS {
            return false;
        }
        if self.sitemaps_enough() {
            self.sitemap_queue.clear();
            return false;
        }
        let mut waiting = VecDeque::new();
        let mut opened = false;
        while let Some(file) = self.sitemap_queue.pop_front() {
            let Some(origin) = Origin::of(&file.url) else {
                continue;
            };
            let due = file.not_before.is_none_or(|at| at <= Instant::now());
            if !due || self.active() >= MAX_IN_FLIGHT {
                waiting.push_back(file);
                continue;
            }
            let ready = self.pace.ready(&origin) && self.session.host_limits().ready(&origin);
            if !ready {
                waiting.push_back(file);
                continue;
            }
            if self.respect.is_some() {
                match self.sites.get(&origin) {
                    Some(Site::Ready(_, read)) if read.elapsed() >= ROBOTS_MAX_AGE => {
                        self.read_robots(origin, file.url.clone());
                        waiting.push_back(file);
                        opened = true;
                        continue;
                    }
                    Some(Site::Ready(robots, _)) if !robots.allows(&file.url) => {
                        self.stats.skip(SkipReason::Robots);
                        continue;
                    }
                    Some(Site::Ready(..)) => {}
                    Some(Site::Pending) => {
                        waiting.push_back(file);
                        continue;
                    }
                    None => {
                        self.read_robots(origin, file.url.clone());
                        waiting.push_back(file);
                        opened = true;
                        continue;
                    }
                }
            }
            self.sitemaps_in_flight += 1;
            let session = self.session.clone();
            let (retry, trusted) = (self.page_retry.clone(), self.trusted.clone());
            self.tasks.push(Box::pin(async move {
                let read =
                    sitemap::read(session, file.url.clone(), retry, file.attempts, trusted).await;
                Event::Sitemap { file, read }
            }));
            opened = true;
            break;
        }
        waiting.append(&mut self.sitemap_queue);
        self.sitemap_queue = waiting;
        opened
    }

    pub(super) fn sitemap_read(&mut self, mut file: SitemapFile, read: SitemapRead) {
        self.sitemaps_in_flight -= 1;
        let found = match read {
            SitemapRead::Found(found) => found,
            SitemapRead::Moved(to) => return self.queue_sitemap(to, Some(file.url)),
            SitemapRead::Later(wait) => {
                let Some(at) = Instant::now().checked_add(wait) else {
                    return self.stats.error();
                };
                file.attempts += 1;
                file.not_before = Some(at);
                self.sitemap_queue.push_back(file);
                return self.set_alarm(at);
            }
            SitemapRead::Failed => return self.stats.error(),
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
