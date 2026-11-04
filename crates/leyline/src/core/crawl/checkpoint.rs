use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, atomic::Ordering};
use std::time::SystemTime;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use url::Url;

use super::frontier::{Frontier, Queued};
use super::page::CrawlStats;
use super::sites::SitemapFile;
use super::visit::Held;
use super::{Crawl, Page, Respect};
use crate::core::config::{HostLimits, Origin};
use crate::core::saved::FileVersion;
use crate::core::{Error, Kind, Result, Session};
use crate::util::atomic::{self, CallerWaits, FileMode};

const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: FileVersion,
    frontier: Frontier,
    respect: Option<Respect>,
    agent: String,
    max_body: Option<usize>,
    max_pages: Option<usize>,
    pace: HostLimits,
    trusted: Option<Url>,
    ready: VecDeque<Page>,
    kept: HashSet<String>,
    texts: HashSet<u64>,
    held: HashMap<String, Vec<Held>>,
    waiting_on: HashMap<String, String>,
    settled: HashSet<String>,
    stats: CrawlStats,
    delayed: Vec<(SystemTime, Queued)>,
    sitemaps: Vec<(Url, u32, Option<SystemTime>)>,
    sitemap_files: HashSet<String>,
    pauses: Vec<(Origin, SystemTime)>,
}

impl Crawl {
    /// Drains current fetches and atomically saves pending URLs, buffered pages, policies, and duplicate history.
    pub async fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        self.start();
        while let Some(event) = self.tasks.next().await {
            self.handle(event);
        }
        let saved = Checkpoint::capture(self)?;
        let caller = CallerWaits::default();
        let waiting = Arc::clone(&caller.0);
        let written = tokio::task::spawn_blocking(move || {
            let bytes = serde_json::to_vec(&saved).map_err(Error::from_json)?;
            if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
                return Err(too_large());
            }
            if !waiting.load(Ordering::Acquire) {
                return Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into());
            }
            atomic::write_atomic(&path, &bytes, FileMode::Private)
        })
        .await
        .map_err(|error| Error::from(std::io::Error::other(error)))?;
        drop(caller);
        written
    }

    /// Loads a checkpoint with a supplied session and rechecks robots.txt before sending new requests.
    pub fn load(session: &Session, path: impl AsRef<Path>) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_CHECKPOINT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
            return Err(too_large());
        }
        let saved: Checkpoint = serde_json::from_slice(&bytes).map_err(Error::from_json)?;
        saved.restore(session)
    }
}

impl Checkpoint {
    fn capture(crawl: &Crawl) -> Result<Self> {
        let ready = crawl
            .ready
            .iter()
            .map(|page| {
                page.as_ref().cloned().map_err(|_| {
                    Error::config("consume buffered crawl errors before saving a checkpoint")
                })
            })
            .collect::<Result<_>>()?;
        let origins: HashSet<_> = crawl
            .frontier
            .origins()
            .cloned()
            .chain(
                crawl
                    .sitemap_queue
                    .iter()
                    .filter_map(|file| Origin::of(&file.url)),
            )
            .collect();
        let pauses = origins
            .into_iter()
            .filter_map(|origin| {
                crawl
                    .pace
                    .ready_at(&origin)
                    .max(crawl.session.host_limits().ready_at(&origin))
                    .map(|at| Ok((origin, wall_time(at)?)))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            version: FileVersion::default(),
            frontier: crawl.frontier.clone(),
            respect: crawl.respect,
            agent: crawl.agent.clone(),
            max_body: crawl.max_body,
            max_pages: crawl.max_pages,
            pace: crawl.pace.clone(),
            trusted: crawl.trusted.clone(),
            ready,
            pauses,
            kept: crawl.kept.clone(),
            texts: crawl.texts.clone(),
            held: crawl.held.clone(),
            waiting_on: crawl.waiting_on.clone(),
            settled: crawl.settled.clone(),
            stats: crawl.stats.clone(),
            delayed: crawl
                .clock
                .delayed
                .iter()
                .map(|(at, queued)| Ok((wall_time(*at)?, queued.clone())))
                .collect::<Result<_>>()?,
            sitemaps: crawl
                .sitemap_queue
                .iter()
                .map(|file| {
                    Ok((
                        file.url.clone(),
                        file.attempts,
                        file.not_before.map(wall_time).transpose()?,
                    ))
                })
                .collect::<Result<_>>()?,
            sitemap_files: crawl.sitemap_files.clone(),
        })
    }

    fn restore(self, session: &Session) -> Result<Crawl> {
        self.frontier.validate()?;
        if self.stats.pages() > self.kept.len() as u64 {
            return Err(Error::config(
                "checkpoint page count exceeds its duplicate history",
            ));
        }
        if let Some(message) = self.pace.config_error() {
            return Err(Error::config(message));
        }
        let mut crawl = Crawl::new(session, Vec::new());
        crawl.frontier = self.frontier;
        crawl.respect = self.respect;
        crawl.agent = self.agent;
        crawl.max_body = self.max_body;
        crawl.max_pages = self.max_pages;
        crawl.limits = Some(self.pace);
        crawl.trusted = self.trusted;
        crawl.ready = self.ready.into_iter().map(Ok).collect();
        crawl.kept = self.kept;
        crawl.texts = self.texts;
        crawl.held = self.held;
        crawl.holding = crawl.held.values().map(Vec::len).sum();
        crawl.waiting_on = self.waiting_on;
        crawl.settled = self.settled;
        crawl.stats = self.stats;
        crawl.clock.delayed = self
            .delayed
            .into_iter()
            .map(|(at, queued)| Ok((monotonic(at)?, queued)))
            .collect::<Result<_>>()?;
        crawl.sitemap_queue = self
            .sitemaps
            .into_iter()
            .map(|(url, attempts, at)| {
                Ok(SitemapFile {
                    url,
                    attempts,
                    not_before: at.map(monotonic).transpose()?,
                })
            })
            .collect::<Result<_>>()?;
        crawl.sitemap_files = self.sitemap_files;
        crawl.start();
        for (origin, at) in self.pauses {
            crawl.pace.pause_until(origin, monotonic(at)?);
        }
        Ok(crawl)
    }
}

fn wall_time(at: Instant) -> Result<SystemTime> {
    SystemTime::now()
        .checked_add(at.saturating_duration_since(Instant::now()))
        .ok_or_else(bad_time)
}
fn monotonic(at: SystemTime) -> Result<Instant> {
    Instant::now()
        .checked_add(at.duration_since(SystemTime::now()).unwrap_or_default())
        .ok_or_else(bad_time)
}
fn bad_time() -> Error {
    Error::config("checkpoint wait is outside the supported clock range")
}
fn too_large() -> Error {
    Error::new(Kind::BodyLimit).with_message("crawl checkpoint exceeds the size limit")
}
