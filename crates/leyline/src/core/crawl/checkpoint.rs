use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use url::Url;

use super::frontier::{Frontier, Queued};
use super::page::CrawlStats;
use super::sites::SitemapFile;
use super::visit::Dedup;
use super::{Crawl, Page, Settings};
use crate::core::atomic::{self, FileMode};
use crate::core::config::{HostLimits, Origin};
use crate::core::session::Session;
use crate::error::{Error, Kind, Result};
use crate::util::version::{self, Versioned};

const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CheckpointVersion {
    #[default]
    V1,
}

impl Versioned for CheckpointVersion {
    fn number(self) -> u32 {
        match self {
            CheckpointVersion::V1 => 1,
        }
    }

    fn from_number(number: u32) -> Option<Self> {
        match number {
            1 => Some(CheckpointVersion::V1),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    #[serde(with = "version")]
    version: CheckpointVersion,
    frontier: Frontier,
    #[serde(flatten)]
    settings: Settings,
    pace: HostLimits,
    trusted: Option<Url>,
    ready: VecDeque<Page>,
    #[serde(flatten)]
    dedup: Dedup,
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
        atomic::blocking(move |waiting| {
            let bytes = serde_json::to_vec(&saved).map_err(Error::from_json)?;
            if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
                return Err(too_large());
            }
            waiting.check()?;
            atomic::write_atomic(&path, &bytes, FileMode::Private)
        })
        .await
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
                page.as_ref().cloned().map_err(|buffered| {
                    Error::config(format!(
                        "consume buffered crawl errors before saving a checkpoint: {buffered}"
                    ))
                })
            })
            .collect::<Result<_>>()?;
        let origins: HashSet<_> = crawl
            .frontier
            .origins()
            .cloned()
            .chain(
                crawl
                    .sitemaps
                    .queue
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
            version: CheckpointVersion::default(),
            frontier: crawl.frontier.clone(),
            settings: crawl.settings.clone(),
            pace: crawl.pace.limits().clone(),
            trusted: crawl.trusted.clone(),
            ready,
            dedup: crawl.dedup.clone(),
            stats: crawl.stats.clone(),
            delayed: crawl
                .clock
                .delayed
                .iter()
                .map(|(at, queued)| Ok((wall_time(*at)?, queued.clone())))
                .collect::<Result<_>>()?,
            sitemaps: crawl
                .sitemaps
                .queue
                .iter()
                .map(|file| {
                    Ok((
                        file.url.clone(),
                        file.attempts,
                        file.not_before.map(wall_time).transpose()?,
                    ))
                })
                .collect::<Result<_>>()?,
            sitemap_files: crawl.sitemaps.files.clone(),
            pauses,
        })
    }

    fn restore(self, session: &Session) -> Result<Crawl> {
        self.frontier.validate()?;
        if self.stats.pages() > self.dedup.kept() as u64 {
            return Err(Error::config(
                "checkpoint page count exceeds its duplicate history",
            ));
        }
        self.pace.validate()?;
        let mut crawl = Crawl::new(session, Vec::new());
        crawl.frontier = self.frontier;
        crawl.settings = self.settings;
        crawl.limits = Some(self.pace);
        crawl.trusted = self.trusted;
        crawl.ready = self.ready.into_iter().map(Ok).collect();
        crawl.dedup = self.dedup;
        crawl.stats = self.stats;
        crawl.clock.delayed = self
            .delayed
            .into_iter()
            .map(|(at, queued)| Ok((monotonic(at)?, queued)))
            .collect::<Result<_>>()?;
        crawl.sitemaps.queue = self
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
        crawl.sitemaps.files = self.sitemap_files;
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
