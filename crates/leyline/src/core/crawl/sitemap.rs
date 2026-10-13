use url::Url;

use crate::core::config::CompressionConfig;
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

pub(super) async fn read(session: Session, url: Url) -> Sitemap {
    let Ok(response) = session.get(url.as_str()).stream().send().await else {
        return Sitemap::default();
    };
    if !response.status().is_success() {
        return Sitemap::default();
    }
    let Ok(bytes) = response.read_until(MAX_BYTES, |_, _| false).await else {
        return Sitemap::default();
    };
    let bytes = if bytes.starts_with(&GZIP_MAGIC) {
        let plain = gunzip(&bytes);
        drop(bytes);
        plain
    } else {
        bytes
    };
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned());
    html::sitemap(&text)
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let limit = Some(MAX_BYTES as u64);
    let Ok(mut decoder) = Decoder::capped(Some(GZIP), &CompressionConfig::new(), limit) else {
        return out;
    };
    let decoded = decoder
        .feed(bytes, &mut out)
        .and_then(|()| decoder.finish(&mut out));
    if let Err(err) = decoded {
        tracing::debug!(%err, kept = out.len(), "sitemap gzip ended early");
    }
    out
}
