mod fetch;
mod pattern;

use std::collections::HashSet;
use std::time::Duration;

use url::Url;

#[cfg(feature = "html")]
pub(crate) use pattern::normalize;
pub(crate) use pattern::{Pattern, path_bytes};

const ROBOTS_PATH: &str = "/robots.txt";
const BYTE_ORDER_MARK: char = '\u{feff}';
const ANY_AGENT: &str = "*";

/// A use of fetched content that robots.txt content signals can allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Purpose {
    /// Building a search index and showing links or snippets.
    Search,
    /// Feeding content to an AI model as input, such as for grounding answers.
    AiInput,
    /// Training or fine-tuning an AI model.
    AiTrain,
}

const USAGE_LABELS: [(&str, Purpose); 3] = [
    ("search", Purpose::Search),
    ("ai-use", Purpose::AiInput),
    ("train-ai", Purpose::AiTrain),
];

const SIGNAL_VALUES: [(&str, bool); 2] = [("yes", true), ("no", false)];
const USAGE_VALUES: [(&str, bool); 2] = [("y", true), ("n", false)];

impl Purpose {
    const ALL: [Purpose; 3] = [Purpose::Search, Purpose::AiInput, Purpose::AiTrain];

    /// Returns the content-signal key: `search`, `ai-input`, or `ai-train`.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Purpose::Search => "search",
            Purpose::AiInput => "ai-input",
            Purpose::AiTrain => "ai-train",
        }
    }

    fn parse(key: &str) -> Option<Purpose> {
        lookup(&Purpose::ALL.map(|purpose| (purpose.id(), purpose)), key)
    }

    fn from_label(label: &str) -> Option<Purpose> {
        lookup(&USAGE_LABELS, label)
    }
}

pub(crate) fn lookup<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key.trim()))
        .map(|(_, value)| *value)
}

/// Where a `Robots` value came from, which decides the default when no rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RobotsSource {
    /// The rules were parsed from a fetched robots.txt.
    Parsed,
    /// The server returned a non-success status other than 5xx or 429, so every path is allowed.
    Unavailable,
    /// The fetch failed, timed out, or returned 5xx or 429, so every path except robots.txt is disallowed.
    Unreachable,
}

/// The robots.txt rules that apply to one user agent, with crawl delay, sitemaps, and content signals.
#[derive(Debug, Clone)]
pub struct Robots {
    source: RobotsSource,
    rules: Vec<Rule>,
    crawl_delay: Option<Duration>,
    sitemaps: Vec<String>,
    uses: Vec<UseRule>,
}

#[derive(Debug, Clone)]
struct UseRule {
    pattern: Option<Pattern>,
    purpose: Purpose,
    allowed: bool,
}

#[derive(Debug, Clone)]
struct Rule {
    pattern: Pattern,
    allow: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    UserAgent,
    Allow,
    Disallow,
    CrawlDelay,
    Sitemap,
    ContentSignal,
    ContentUsage,
}

const FIELDS: [(&str, Field); 11] = [
    ("user-agent", Field::UserAgent),
    ("useragent", Field::UserAgent),
    ("user agent", Field::UserAgent),
    ("allow", Field::Allow),
    ("disallow", Field::Disallow),
    ("dissallow", Field::Disallow),
    ("crawl-delay", Field::CrawlDelay),
    ("sitemap", Field::Sitemap),
    ("site-map", Field::Sitemap),
    ("content-signal", Field::ContentSignal),
    ("content-usage", Field::ContentUsage),
];

impl Field {
    fn parse(key: &str) -> Option<Field> {
        lookup(&FIELDS, key)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Agent {
    Any,
    Named(String),
}

impl Agent {
    fn parse(token: &str) -> Option<Agent> {
        if token.trim() == ANY_AGENT {
            return Some(Agent::Any);
        }
        let name = product_token(token).to_ascii_lowercase();
        (!name.is_empty()).then_some(Agent::Named(name))
    }
}

pub(crate) fn product_token(agent: &str) -> &str {
    let agent = agent.trim();
    let end = agent
        .find(|c: char| !(c.is_ascii_alphabetic() || matches!(c, '_' | '-')))
        .unwrap_or(agent.len());
    &agent[..end]
}

#[derive(Default)]
struct Group {
    agents: HashSet<Agent>,
    rules: Vec<Rule>,
    crawl_delay: Option<Duration>,
    uses: Vec<UseRule>,
}

impl Group {
    fn apply(&mut self, field: Field, value: &str) {
        match field {
            Field::Allow | Field::Disallow => {
                if let Some(pattern) = Pattern::parse(value) {
                    let allow = field == Field::Allow;
                    self.rules.push(Rule { pattern, allow });
                }
            }
            Field::CrawlDelay => {
                let seconds = value
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s >= 0.0);
                self.crawl_delay = self
                    .crawl_delay
                    .or(seconds.map(|s| Duration::try_from_secs_f64(s).unwrap_or(Duration::MAX)));
            }
            Field::ContentSignal => {
                let prefs = value
                    .split(',')
                    .filter_map(|entry| pref(entry, signal_key, &SIGNAL_VALUES));
                self.uses.extend(prefs.map(|(purpose, allowed)| UseRule {
                    pattern: None,
                    purpose,
                    allowed,
                }));
            }
            Field::ContentUsage => {
                let (pattern, prefs) = scoped_usage(value);
                self.uses
                    .extend(usage_prefs(prefs).map(|(purpose, allowed)| UseRule {
                        pattern: pattern.clone(),
                        purpose,
                        allowed,
                    }));
            }
            Field::UserAgent | Field::Sitemap => {}
        }
    }
}

fn signal_key(key: &str) -> Option<Purpose> {
    Purpose::parse(key.trim())
}

fn pref(
    entry: &str,
    purpose: fn(&str) -> Option<Purpose>,
    values: &[(&str, bool)],
) -> Option<(Purpose, bool)> {
    let (key, value) = entry.split_once('=')?;
    let value = value.split(';').next().unwrap_or_default();
    Some((purpose(key)?, lookup(values, value)?))
}

fn usage_prefs(prefs: &str) -> impl Iterator<Item = (Purpose, bool)> + '_ {
    prefs
        .split(',')
        .filter_map(|entry| pref(entry, Purpose::from_label, &USAGE_VALUES))
}

fn scoped_usage(value: &str) -> (Option<Pattern>, &str) {
    match value.split_once(char::is_whitespace) {
        Some((path, prefs)) if path.starts_with(['/', '*']) => (Pattern::parse(path), prefs),
        _ => (None, value),
    }
}

#[cfg(feature = "html")]
pub(crate) fn header_uses(value: &str) -> impl Iterator<Item = (Purpose, bool)> + '_ {
    usage_prefs(value)
}

impl Robots {
    /// Parses robots.txt text and keeps the groups for `agent`, falling back to the `*` groups.
    #[must_use]
    pub fn parse(text: &str, agent: &str) -> Robots {
        let mut groups: Vec<Group> = Vec::new();
        let mut sitemaps = Vec::new();
        let mut collecting_agents = false;
        for line in text.trim_start_matches(BYTE_ORDER_MARK).split(['\n', '\r']) {
            let line = line.split('#').next().unwrap_or_default();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let Some(field) = Field::parse(key.trim()) else {
                continue;
            };
            let value = value.trim();
            match field {
                Field::UserAgent => {
                    if !collecting_agents {
                        groups.push(Group::default());
                        collecting_agents = true;
                    }
                    if let (Some(group), Some(agent)) = (groups.last_mut(), Agent::parse(value)) {
                        group.agents.insert(agent);
                    }
                }
                Field::Sitemap if !value.is_empty() => sitemaps.push(value.to_owned()),
                Field::Sitemap => {}
                field => {
                    collecting_agents = false;
                    if let Some(group) = groups.last_mut() {
                        group.apply(field, value);
                    }
                }
            }
        }
        let wanted = match Agent::parse(agent) {
            Some(named) if groups.iter().any(|group| group.agents.contains(&named)) => named,
            _ => Agent::Any,
        };
        let mut robots = Robots::empty(RobotsSource::Parsed);
        robots.sitemaps = sitemaps;
        for group in groups
            .into_iter()
            .filter(|group| group.agents.contains(&wanted))
        {
            robots.rules.extend(group.rules);
            robots.crawl_delay = robots.crawl_delay.max(group.crawl_delay);
            robots.uses.extend(group.uses);
        }
        robots
    }

    pub(crate) fn empty(source: RobotsSource) -> Robots {
        Robots {
            source,
            rules: Vec::new(),
            crawl_delay: None,
            sitemaps: Vec::new(),
            uses: Vec::new(),
        }
    }

    /// Returns whether `url` may be crawled, using the longest matching rule; robots.txt itself is always allowed.
    #[must_use]
    pub fn allows(&self, url: &Url) -> bool {
        if url.path() == ROBOTS_PATH {
            return true;
        }
        match self.source {
            RobotsSource::Unavailable => true,
            RobotsSource::Unreachable => false,
            RobotsSource::Parsed => {
                let path = path_bytes(url);
                self.rules
                    .iter()
                    .filter(|rule| rule.pattern.matches(&path))
                    .max_by_key(|rule| (rule.pattern.specificity(), rule.allow))
                    .is_none_or(|rule| rule.allow)
            }
        }
    }

    /// Returns whether content at `url` may be used for `purpose`; allowed when no content signal matches.
    #[must_use]
    pub fn allows_use(&self, url: &Url, purpose: Purpose) -> bool {
        let path = path_bytes(url);
        self.uses
            .iter()
            .filter(|rule| rule.purpose == purpose)
            .filter_map(|rule| match &rule.pattern {
                None => Some((0, rule.allowed)),
                Some(pattern) if pattern.matches(&path) => {
                    Some((pattern.specificity(), rule.allowed))
                }
                Some(_) => None,
            })
            .max_by_key(|(specificity, allowed)| (*specificity, !allowed))
            .is_none_or(|(_, allowed)| allowed)
    }

    /// Returns the largest crawl delay among the matched groups, or `None` when none sets one.
    #[must_use]
    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// Returns every sitemap URL listed in the file, regardless of user-agent group.
    #[must_use]
    pub fn sitemaps(&self) -> &[String] {
        &self.sitemaps
    }

    /// Returns where these rules came from.
    #[must_use]
    pub fn source(&self) -> RobotsSource {
        self.source
    }
}
