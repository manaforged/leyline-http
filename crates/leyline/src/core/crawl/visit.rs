use std::collections::{HashMap, HashSet};

use http::HeaderMap;
use sha2::{Digest, Sha256};
use url::Url;

use super::fetch::{CONTENT_USAGE, Loaded};
use super::frontier::{Queued, page_key};
use super::{Crawl, MIN_PRINT_WORDS, NOFOLLOW, Page, SkipReason};
use crate::core::robots::{Purpose, usage_prefs};
use crate::html::{self, Directives};

impl Crawl {
    fn refused_by_header(&self, headers: &HeaderMap) -> bool {
        self.purpose().is_some_and(|purpose| {
            headers
                .get_all(CONTENT_USAGE)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(usage_prefs)
                .any(|(usage, allowed)| usage == purpose && !allowed)
        })
    }

    fn excluded(&self, rules: Directives, headers: &HeaderMap) -> Option<SkipReason> {
        if self.settings.respect.is_some() && !rules.index() {
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
        let rules = Directives::read(&html, &headers, &self.settings.agent);
        let may_follow = self.settings.respect.is_none() || rules.follow();
        if may_follow {
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
            page: Page::new(url, depth, status, html, text),
        };
        if let (Some(key), Some(link)) = (held.claimed.clone(), canonical)
            && may_follow
            && key != held.own
            && !self.dedup.kept.contains(&key)
            && !self.dedup.settled.contains(&key)
        {
            self.frontier.add(Queued::link(link, depth));
            self.dedup.waiting_on.insert(held.own.clone(), key.clone());
            self.dedup.held.entry(key).or_default().push(held);
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
        let duplicate = self.dedup.kept.contains(&own)
            || claimed
                .as_ref()
                .is_some_and(|key| self.dedup.kept.contains(key))
            || print.is_some_and(|print| self.dedup.texts.contains(&print));
        if duplicate {
            self.stats.skip(SkipReason::Duplicate);
            return;
        }
        self.dedup.kept.insert(own);
        if let Some(key) = claimed {
            self.dedup.kept.insert(key);
        }
        if let Some(print) = print {
            self.dedup.texts.insert(print);
        }
        self.stats.page();
        self.ready.push_back(Ok(page));
    }

    pub(super) fn settle(&mut self, key: String) {
        let kept = self.dedup.kept.contains(&key);
        let waiting = self.dedup.held.remove(&key).unwrap_or_default();
        if let Some(next) = self.dedup.waiting_on.get(&key).cloned() {
            self.dedup.held.entry(next).or_default().extend(waiting);
            self.dedup.settled.insert(key);
            return;
        }
        self.dedup.settled.insert(key);
        for held in waiting {
            self.dedup.waiting_on.remove(&held.own);
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
        let waiting = self.dedup.held.remove(&from).unwrap_or_default();
        self.dedup.settled.insert(from);
        if waiting.is_empty() {
            return;
        }
        let (cycle, onward): (Vec<Held>, Vec<Held>) =
            waiting.into_iter().partition(|held| held.own == to);
        for held in cycle {
            self.dedup.waiting_on.remove(&held.own);
            self.keep(held);
        }
        for held in &onward {
            self.dedup.waiting_on.insert(held.own.clone(), to.clone());
        }
        self.dedup
            .held
            .entry(to.clone())
            .or_default()
            .extend(onward);
        if self.dedup.settled.contains(&to) {
            self.settle(to);
        }
    }

    pub(super) fn release_held(&mut self) -> bool {
        self.dedup.waiting_on.clear();
        let keys: Vec<String> = self.dedup.held.keys().cloned().collect();
        for key in &keys {
            self.settle(key.clone());
        }
        !keys.is_empty()
    }

    fn follow(&mut self, base: &Url, html: &str, depth: usize) {
        for anchor in html::links(html) {
            let nofollow = self.settings.respect.is_some()
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

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Dedup {
    kept: HashSet<String>,
    texts: HashSet<u64>,
    held: HashMap<String, Vec<Held>>,
    waiting_on: HashMap<String, String>,
    settled: HashSet<String>,
}

impl Dedup {
    pub(super) fn holding(&self) -> usize {
        self.held.values().map(Vec::len).sum()
    }

    pub(super) fn kept(&self) -> usize {
        self.kept.len()
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Held {
    own: String,
    claimed: Option<String>,
    print: Option<u64>,
    page: Page,
}

fn uses_ai(purpose: Purpose) -> bool {
    matches!(purpose, Purpose::AiInput | Purpose::AiTrain)
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = Sha256::new();
    for word in text.split_whitespace() {
        hasher.update((word.len() as u64).to_be_bytes());
        hasher.update(word.as_bytes());
    }
    let hash = hasher.finalize();
    u64::from_be_bytes(
        hash[..8]
            .try_into()
            .expect("invariant: SHA-256 has at least eight bytes"),
    )
}
