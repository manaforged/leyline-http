use std::hash::{DefaultHasher, Hash, Hasher};

use http::HeaderMap;
use url::Url;

use super::fetch::{CONTENT_USAGE, Loaded};
use super::frontier::{Queued, page_key};
use super::{ANY_AGENT, Crawl, CrawlPage, MIN_PRINT_WORDS, NOFOLLOW, SkipReason};
use crate::core::robots::{Purpose, header_uses};
use crate::html::{self, Directives};

impl Crawl {
    fn refused_by_header(&self, headers: &HeaderMap) -> bool {
        self.purpose().is_some_and(|purpose| {
            headers
                .get_all(CONTENT_USAGE)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(header_uses)
                .any(|(usage, allowed)| usage == purpose && !allowed)
        })
    }

    fn excluded(&self, rules: Directives, headers: &HeaderMap) -> Option<SkipReason> {
        if self.respect.is_some() && !rules.index() {
            Some(SkipReason::NoIndex)
        } else if !rules.ai() && self.purpose().is_some_and(uses_ai) {
            Some(SkipReason::NoAi)
        } else if self.refused_by_header(headers) {
            Some(SkipReason::Signal)
        } else {
            None
        }
    }

    pub(super) fn page(&mut self, depth: usize, loaded: Loaded) {
        let Loaded {
            url,
            status,
            headers,
            html,
        } = loaded;
        let base = html::base(&html)
            .and_then(|href| url.join(&href).ok())
            .unwrap_or_else(|| url.clone());
        let rules = Directives::read(&html, &headers, ANY_AGENT);
        if self.respect.is_none() || rules.follow() {
            self.follow(&base, &html, depth + 1);
        }
        if let Some(reason) = self.excluded(rules, &headers) {
            self.stats.skip(reason);
            return;
        }
        let canonical = html::canonical(&html)
            .and_then(|href| base.join(&href).ok())
            .filter(|link| self.frontier.in_scope(link));
        let own = page_key(&url);
        let claimed = canonical.as_ref().map(page_key);
        let text = html::text(&html);
        let print = (text.split_whitespace().nth(MIN_PRINT_WORDS - 1).is_some())
            .then(|| fingerprint(&text));
        let held = Held {
            own,
            claimed,
            print,
            page: CrawlPage::new(url, depth, status, html, text),
        };
        if let (Some(key), Some(link)) = (held.claimed.clone(), canonical)
            && key != held.own
            && !self.kept.contains(&key)
            && !self.settled.contains(&key)
        {
            self.frontier.add(Queued::link(link, depth));
            self.waiting_on.insert(held.own.clone(), key.clone());
            self.held.entry(key).or_default().push(held);
            self.holding += 1;
            return;
        }
        self.keep(held);
    }

    fn keep(&mut self, held: Held) {
        let Held {
            own,
            claimed,
            print,
            page,
        } = held;
        let duplicate = self.kept.contains(&own)
            || claimed.as_ref().is_some_and(|key| self.kept.contains(key))
            || print.is_some_and(|print| self.texts.contains(&print));
        if duplicate {
            self.stats.skip(SkipReason::Duplicate);
            return;
        }
        self.kept.insert(own);
        if let Some(key) = claimed {
            self.kept.insert(key);
        }
        if let Some(print) = print {
            self.texts.insert(print);
        }
        self.stats.page();
        self.ready.push_back(Ok(page));
    }

    pub(super) fn settle(&mut self, key: String) {
        let kept = self.kept.contains(&key);
        let waiting = self.held.remove(&key).unwrap_or_default();
        if let Some(next) = self.waiting_on.get(&key).cloned() {
            self.held.entry(next).or_default().extend(waiting);
            self.settled.insert(key);
            return;
        }
        self.settled.insert(key);
        for held in waiting {
            self.holding -= 1;
            self.waiting_on.remove(&held.own);
            if kept {
                self.stats.skip(SkipReason::Duplicate);
            } else {
                self.keep(held);
            }
        }
    }

    pub(super) fn forward(&mut self, from: String, to: String) {
        if from == to {
            return;
        }
        let waiting = self.held.remove(&from).unwrap_or_default();
        self.settled.insert(from);
        if waiting.is_empty() {
            return;
        }
        let (cycle, onward): (Vec<Held>, Vec<Held>) =
            waiting.into_iter().partition(|held| held.own == to);
        for held in cycle {
            self.holding -= 1;
            self.waiting_on.remove(&held.own);
            self.keep(held);
        }
        for held in &onward {
            self.waiting_on.insert(held.own.clone(), to.clone());
        }
        self.held.entry(to.clone()).or_default().extend(onward);
        if self.settled.contains(&to) {
            self.settle(to);
        }
    }

    pub(super) fn release_held(&mut self) -> bool {
        self.waiting_on.clear();
        let keys: Vec<String> = self.held.keys().cloned().collect();
        for key in &keys {
            self.settle(key.clone());
        }
        !keys.is_empty()
    }

    fn follow(&mut self, base: &Url, html: &str, depth: usize) {
        for anchor in html::links(html) {
            let nofollow = self.respect.is_some()
                && anchor.rel().is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case(NOFOLLOW))
                });
            if let (false, Ok(url)) = (nofollow, base.join(anchor.href())) {
                self.frontier.add(Queued::link(url, depth));
            }
        }
    }
}

pub(super) struct Held {
    own: String,
    claimed: Option<String>,
    print: Option<u64>,
    page: CrawlPage,
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
