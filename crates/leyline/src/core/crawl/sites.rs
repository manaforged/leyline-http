use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::frontier::{Queued, page_key};
use super::{ANY_AGENT, Crawl, Event, MAX_ATTEMPTS, MAX_IN_FLIGHT, SkipReason, Task, sitemap};
use crate::core::config::{Origin, normal_host};
use crate::core::robots::{Robots, RobotsSource};

const ROBOTS_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);
const MAX_SITEMAP_FILES: usize = 50;
const MAX_SITEMAP_TASKS: usize = 2;
const ROBOTS_RETRY: Duration = Duration::from_secs(5);

pub(super) enum Site {
    Pending,
    Ready(Robots, Instant),
}

pub(super) struct SitemapFile {
    pub(super) url: Url,
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
        if self.session.is_shut_down() {
            self.frontier.clear();
            self.sitemap_queue.clear();
            return;
        }
        while self.tasks.len() < MAX_IN_FLIGHT {
            if self.open_sitemap() {
                continue;
            }
            if self.budget_spent() {
                break;
            }
            let (sites, busy, pace) = (&self.sites, &self.busy, &self.pace);
            let Some(queued) = self.frontier.pop_where(|origin| {
                let cap = pace.max_in_flight_for(origin).unwrap_or(MAX_IN_FLIGHT);
                let busy = busy.get(origin).copied().unwrap_or(0);
                !matches!(sites.get(origin), Some(Site::Pending))
                    && busy < cap
                    && (busy == 0 || pace.ready(origin))
            }) else {
                break;
            };
            let Some(origin) = Origin::of(&queued.url) else {
                continue;
            };
            let known = match self.sites.get(&origin) {
                Some(Site::Ready(robots, read)) if read.elapsed() < ROBOTS_MAX_AGE => {
                    Some(self.verdict(robots, &queued.url))
                }
                _ => None,
            };
            match known {
                None => {
                    self.tasks.push(self.robots_task(
                        origin.clone(),
                        queued.url.clone(),
                        Duration::ZERO,
                    ));
                    self.sites.insert(origin, Site::Pending);
                    self.frontier.push_front(queued);
                }
                Some(Some(reason)) => {
                    self.stats.skip(reason);
                    self.settle(page_key(&queued.url));
                }
                Some(None) => self.fetch(origin, queued),
            }
        }
    }

    fn robots_task(&self, origin: Origin, url: Url, after: Duration) -> Task {
        let session = self.session.clone();
        Box::pin(async move {
            session.deadline(None).sleep(after).await;
            let robots = match session.robots(url.as_str(), ANY_AGENT).await {
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
        if robots.source() == RobotsSource::Unreachable {
            let tries = self.robots_tries.entry(origin.clone()).or_default();
            *tries += 1;
            if *tries < MAX_ATTEMPTS {
                let after = ROBOTS_RETRY * u32::from(*tries);
                self.tasks.push(self.robots_task(origin, url, after));
                return;
            }
        }
        self.robots_tries.remove(&origin);
        for sitemap in sitemap::listed(&url, &robots, self.respect.is_some()) {
            self.queue_sitemap(sitemap, None);
        }
        if self.respect.is_some() {
            let gap = robots.crawl_delay().map(|gap| gap.min(MAX_CRAWL_DELAY));
            self.pace.set_floor(&url, gap);
        }
        self.sites
            .insert(origin, Site::Ready(robots, Instant::now()));
    }

    pub(super) fn queue_sitemap(&mut self, url: Url, parent: Option<Url>) {
        let same_site = parent.as_ref().is_none_or(|parent| {
            parent.host_str().map(normal_host) == url.host_str().map(normal_host)
        });
        if same_site
            && self.sitemap_files.len() < MAX_SITEMAP_FILES
            && self.sitemap_files.insert(url.as_str().to_owned())
        {
            self.sitemap_queue.push_back(SitemapFile { url });
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
        while let Some(file) = self.sitemap_queue.pop_front() {
            let refused = self.respect.is_some()
                && match Origin::of(&file.url).and_then(|origin| self.sites.get(&origin)) {
                    Some(Site::Ready(robots, _)) => !robots.allows(&file.url),
                    _ => true,
                };
            if refused {
                self.stats.skip(SkipReason::Robots);
                continue;
            }
            self.sitemaps_in_flight += 1;
            let session = self.session.clone();
            self.tasks.push(Box::pin(async move {
                let found = sitemap::read(session, file.url.clone()).await;
                Event::Sitemap {
                    url: file.url,
                    found,
                }
            }));
            return true;
        }
        false
    }

    pub(super) fn sitemap_read(&mut self, url: Url, index: bool, locs: Vec<String>) {
        self.sitemaps_in_flight -= 1;
        for loc in locs.iter().filter_map(|loc| Url::parse(loc).ok()) {
            if index {
                self.queue_sitemap(loc, Some(url.clone()));
            } else {
                self.frontier.add(Queued::link(loc, 0));
            }
        }
    }
}
