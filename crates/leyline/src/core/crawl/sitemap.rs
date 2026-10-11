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

pub(super) fn listed(site: &str, robots: &Robots, obey: bool) -> Vec<Url> {
    let listed: Vec<Url> = robots
        .sitemaps()
        .iter()
        .filter_map(|link| Url::parse(link).ok())
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    Url::parse(site)
        .and_then(|origin| origin.join(SITEMAP_PATH))
        .ok()
        .filter(|url| !obey || robots.allows(url))
        .into_iter()
        .collect()
}

pub(super) async fn read(session: Session, url: Url) -> Sitemap {
    let Ok(read) = session
        .get(url.as_str())
        .read_until(MAX_BYTES, |_, _| false)
        .await
    else {
        return Sitemap::default();
    };
    let bytes = if read.bytes.starts_with(&GZIP_MAGIC) {
        gunzip(&read.bytes).unwrap_or_default()
    } else {
        read.bytes
    };
    html::sitemap(&String::from_utf8_lossy(&bytes))
}

fn gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    let limit = Some(MAX_BYTES as u64);
    let mut decoder = Decoder::capped(Some(GZIP), &CompressionConfig::new(), limit).ok()?;
    let mut out = Vec::new();
    decoder.feed(bytes, &mut out).ok()?;
    decoder.finish(&mut out).ok()?;
    Some(out)
}
