use std::time::Duration;

use http::StatusCode;
use http::header::LOCATION;
use url::Url;

use super::fetch::request;

use crate::core::config::CompressionConfig;
use crate::core::retry::{RetryPlan, RetryPolicy};
use crate::core::robots::Robots;
use crate::core::session::Session;
use crate::core::session::decompress::Decoder;
use crate::html::{self, Sitemap};

const SITEMAP_PATH: &str = "/sitemap.xml";
const MAX_BYTES: usize = 50 * 1024 * 1024;
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const GZIP: &str = "gzip";

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
    Failed,
}

pub(super) async fn read(
    session: Session,
    url: Url,
    retry: RetryPolicy,
    attempt: u32,
    trusted: Option<Url>,
) -> SitemapRead {
    let result = request(&session, &url, retry.clone().max_retries(0), trusted)
        .stream()
        .send()
        .await;
    if let RetryPlan::Backoff(wait) = retry.plan(&result, attempt, true, || true) {
        return SitemapRead::Later(wait);
    }
    let Ok(response) = result else {
        return SitemapRead::Failed;
    };
    if response.status().is_redirection() {
        return match response
            .header(LOCATION.as_str())
            .and_then(|to| url.join(to).ok())
        {
            Some(to) => SitemapRead::Moved(to),
            None => SitemapRead::Failed,
        };
    }
    if response.status().is_client_error() && response.status() != StatusCode::TOO_MANY_REQUESTS {
        return SitemapRead::Found(Sitemap::default());
    }
    if !response.status().is_success() {
        return SitemapRead::Failed;
    }
    let Ok(bytes) = response.read_until(MAX_BYTES, |_, _| false).await else {
        return SitemapRead::Failed;
    };
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        let Some(plain) = gunzip(&bytes) else {
            return SitemapRead::Failed;
        };
        drop(bytes);
        plain
    } else {
        bytes
    };
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned());
    SitemapRead::Found(html::sitemap(&text))
}

fn gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let limit = Some(MAX_BYTES as u64);
    let mut decoder = Decoder::capped(Some(GZIP), &CompressionConfig::new(), limit).ok()?;
    let decoded = decoder
        .feed(bytes, &mut out)
        .and_then(|()| decoder.finish(&mut out));
    match decoded {
        Ok(()) => Some(out),
        Err(err) => {
            tracing::debug!(%err, "sitemap gzip did not finish");
            None
        }
    }
}
