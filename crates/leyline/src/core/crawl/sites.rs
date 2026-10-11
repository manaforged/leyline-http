use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::frontier::{Site, site_key};
use super::{ANY_AGENT, Crawl, Event, MAX_IN_FLIGHT, SkipReason, Task, sitemap};
use crate::core::robots::{Robots, RobotsSource};

const ROBOTS_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(30);

enum State {
    Unknown,
    Waiting,
    Stale,
    Known(Option<SkipReason>),
}

impl Crawl {
    fn verdict(&self, robots: &Robots, url: &Url) -> Option<SkipReason> {
        if self.robots && !robots.allows(url) {
            return Some(SkipReason::Robots);
        }
        let refused = self
            .purpose
            .is_some_and(|purpose| !robots.allows_use(url, purpose));
        refused.then_some(SkipReason::Signal)
    }

    fn state(&self, site: &str, url: &Url) -> State {
        match self.frontier.sites.get(site) {
            None => State::Unknown,
            Some(Site::Pending(_)) => State::Waiting,
            Some(Site::Ready(_, read)) if read.elapsed() >= ROBOTS_MAX_AGE => State::Stale,
            Some(Site::Ready(robots, _)) => State::Known(self.verdict(robots, url)),
        }
    }

    pub(super) fn schedule(&mut self) {
        if self.session.is_shut_down() {
            self.frontier.clear();
            return;
        }
        while self.tasks.len() < MAX_IN_FLIGHT && !self.budget_spent() {
            let Some(queued) = self.frontier.pop() else {
                break;
            };
            let site = site_key(&queued.url);
            match self.state(&site, &queued.url) {
                State::Unknown | State::Stale => {
                    self.tasks
                        .push(self.robots_task(site.clone(), queued.url.clone()));
                    self.frontier
                        .sites
                        .insert(site, Site::Pending(vec![queued]));
                }
                State::Waiting => {
                    if let Some(Site::Pending(waiting)) = self.frontier.sites.get_mut(&site) {
                        waiting.push(queued);
                    }
                }
                State::Known(Some(reason)) => self.stats.skip(reason),
                State::Known(None) => self.fetch(queued),
            }
        }
    }

    fn robots_task(&self, site: String, url: Url) -> Task {
        let session = self.session.clone();
        Box::pin(async move {
            let robots = match session.robots(url, ANY_AGENT).await {
                Ok(robots) => robots,
                Err(_) => Robots::empty(RobotsSource::Unreachable),
            };
            Event::Robots {
                site,
                robots: Box::new(robots),
            }
        })
    }

    pub(super) fn robots_read(&mut self, site: String, robots: Robots) {
        for url in sitemap::listed(&site, &robots, self.robots) {
            self.read_sitemap(url);
        }
        if let (true, Some(gap), Ok(origin)) =
            (self.robots, robots.crawl_delay(), Url::parse(&site))
        {
            self.session
                .host_limits()
                .raise_floor(&origin, gap.min(MAX_CRAWL_DELAY));
        }
        let ready = Site::Ready(robots, Instant::now());
        if let Some(Site::Pending(waiting)) = self.frontier.sites.insert(site, ready) {
            self.frontier.resume(waiting);
        }
    }
}
