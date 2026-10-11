use std::hash::{DefaultHasher, Hash, Hasher};

use url::Url;

use super::fetch::Loaded;
use super::frontier::{Queued, page_key};
use super::{ANY_AGENT, Crawl, CrawlPage, NOFOLLOW, SkipReason};

const CONTENT_USAGE: &str = "content-usage";
use crate::core::robots::{Purpose, header_uses};
use crate::html::{self, Directives};

impl Crawl {
    pub(super) fn page(&mut self, depth: usize, loaded: Loaded) {
        let Loaded {
            url,
            status,
            headers,
            html,
        } = loaded;
        let rules = Directives::read(&html, &headers, ANY_AGENT);
        let follows = !self.robots || rules.follow();
        if follows {
            self.follow(&url, &html, depth + 1);
        }
        let refused = self.purpose.is_some_and(|purpose| {
            headers
                .get_all(CONTENT_USAGE)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(header_uses)
                .any(|(usage, allowed)| usage == purpose && !allowed)
        });
        let reason = if self.robots && !rules.index() {
            Some(SkipReason::NoIndex)
        } else if !rules.ai() && self.purpose.is_some_and(uses_ai) {
            Some(SkipReason::NoAi)
        } else if refused {
            Some(SkipReason::Signal)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.stats.skip(reason);
            return;
        }
        let canonical = html::canonical(&html)
            .and_then(|href| url.join(&href).ok())
            .filter(|link| self.frontier.in_scope(link))
            .map_or_else(|| page_key(&url), |link| page_key(&link));
        let fresh_url = !self.kept.contains(&canonical);
        self.kept.insert(page_key(&url));
        self.kept.insert(canonical);
        let text = html::text(&html);
        let fresh_text = self.texts.insert(fingerprint(&text));
        if !(fresh_url && fresh_text) {
            self.stats.skip(SkipReason::Duplicate);
            return;
        }
        self.stats.page();
        self.ready
            .push_back(Ok(CrawlPage::new(url, depth, status, html, text)));
    }

    fn follow(&mut self, base: &Url, html: &str, depth: usize) {
        for anchor in html::links(html) {
            let nofollow = self.robots
                && anchor.rel().is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case(NOFOLLOW))
                });
            if let (false, Ok(url)) = (nofollow, base.join(anchor.href())) {
                self.frontier.add(Queued {
                    url,
                    depth,
                    hops: 0,
                    seed: false,
                });
            }
        }
    }
}

fn uses_ai(purpose: Purpose) -> bool {
    matches!(purpose, Purpose::AiInput | Purpose::AiTrain)
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    for word in text.split_whitespace() {
        word.hash(&mut hasher);
    }
    hasher.finish()
}
