use std::fmt;
use std::time::Duration;

use http::StatusCode;
use url::Url;

use crate::core::BodyRules;
use crate::core::config::Codings;
use crate::core::decompress::Decoder;
use crate::core::retry::Retry;
use crate::core::robots::{Reply, Robots, read_capped};
use crate::core::session::Session;
use crate::error::{Error, Result};
use crate::html::{Scanner, decode};
use crate::profile::Encoding;
use crate::util::{is_redirect_status, lookup, redirect_location};
use crate::wire::Scheme;

const SITEMAP_PATH: &str = "/sitemap.xml";
const MAX_BYTES: usize = 50 * 1024 * 1024;
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Loc,
    Set,
    Index,
}

const FIELDS: [(&str, Field); 3] = [
    ("loc", Field::Loc),
    ("urlset", Field::Set),
    ("sitemapindex", Field::Index),
];

impl Field {
    fn of(name: &str) -> Option<(&str, Field)> {
        let (prefix, local) = name.rsplit_once(':').unwrap_or(("", name));
        Some((prefix, lookup(&FIELDS, local)?))
    }
}

#[derive(Debug, Default)]
pub(super) struct Sitemap {
    pub(super) index: bool,
    pub(super) locs: Vec<String>,
}

impl Sitemap {
    fn parse(document: &str) -> Sitemap {
        if !document.contains('<') {
            return Sitemap::plain(document);
        }
        let mut sitemap = Sitemap::default();
        let mut root: Option<String> = None;
        for tag in Scanner::xml(document).filter(|tag| !tag.closing) {
            let Some((prefix, field)) = Field::of(&tag.name) else {
                continue;
            };
            match field {
                Field::Set | Field::Index => {
                    sitemap.index |= field == Field::Index;
                    root.get_or_insert_with(|| prefix.to_owned());
                }
                Field::Loc if root.as_deref().unwrap_or_default() == prefix => {
                    let loc = decode(tag.text.trim());
                    if !loc.is_empty() {
                        sitemap.locs.push(loc);
                    }
                }
                Field::Loc => {}
            }
        }
        sitemap
    }

    fn plain(document: &str) -> Sitemap {
        let locs = document
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.split_once("://")
                    .is_some_and(|(scheme, _)| Scheme::parse(scheme).is_some())
            })
            .map(str::to_owned)
            .collect();
        Sitemap { index: false, locs }
    }
}

pub(super) fn listed(site: &Url, robots: &Robots, obey: bool) -> Vec<Url> {
    let listed: Vec<Url> = robots
        .sitemaps()
        .iter()
        .filter_map(|link| Url::parse(link).ok())
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    site.join(SITEMAP_PATH)
        .ok()
        .filter(|url| !obey || robots.allows(url))
        .into_iter()
        .collect()
}

pub(super) enum SitemapRead {
    Found(Sitemap),
    Moved(Url),
    Later(Duration),
    Failed(Failure),
}

pub(super) enum Failure {
    Error(Error),
    Status(StatusCode),
    NoLocation,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Error(error) => write!(f, "{error}"),
            Failure::Status(status) => write!(f, "status {}", status.as_u16()),
            Failure::NoLocation => f.write_str("redirect without a usable location"),
        }
    }
}

pub(super) async fn read(
    session: Session,
    url: Url,
    retry: Retry,
    attempt: u32,
    trusted: Option<Url>,
) -> SitemapRead {
    let response = match Reply::send(&session, &url, &retry, attempt, trusted, None).await {
        Reply::Later(wait) => return SitemapRead::Later(wait),
        Reply::Sent(Ok(response)) => response,
        Reply::Sent(Err(error)) => return SitemapRead::Failed(Failure::Error(error)),
    };
    let status = response.status();
    if is_redirect_status(status.as_u16()) {
        return match redirect_location(status.as_u16(), response.headers(), &url) {
            Some(to) => SitemapRead::Moved(to),
            None => SitemapRead::Failed(Failure::NoLocation),
        };
    }
    if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
        return SitemapRead::Found(Sitemap::default());
    }
    if !status.is_success() {
        return SitemapRead::Failed(Failure::Status(status));
    }
    let bytes = match read_capped(&session, response, MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => return SitemapRead::Failed(Failure::Error(error)),
    };
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        let plain = gunzip(&bytes);
        drop(bytes);
        match plain {
            Ok(plain) => plain,
            Err(error) => return SitemapRead::Failed(Failure::Error(error)),
        }
    } else {
        bytes
    };
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned());
    SitemapRead::Found(Sitemap::parse(&text))
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut rules = BodyRules::default();
    rules.max_body_size = MAX_BYTES;
    let mut decoder = Decoder::capped(&Codings::Listed(vec![Encoding::Gzip]), &rules)?;
    decoder.feed(bytes, &mut out)?;
    decoder.finish(&mut out)?;
    Ok(out)
}
