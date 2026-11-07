use std::borrow::Cow;
use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock, Mutex};

use lru::LruCache;

use crate::header_str::HeaderStr;
use crate::profile::Platform;
use crate::util::is_body_header;
use foldhash::HashMap;

use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/header_style.rs"));

mod kind;
pub use kind::RequestKind;

pub struct HeaderContext<'a> {
    pub user_agent: &'a HeaderStr,
    pub sec_ch_ua: &'a HeaderStr,
    pub sec_ch_ua_mobile: &'static str,
    pub sec_ch_ua_platform: &'a str,
    pub accept_language: &'a HeaderStr,
    pub origin: &'a str,
    pub referer: &'a str,
    pub fetch_site: &'static str,
    pub navigation_site: &'static str,
    pub navigation_referer: &'a str,
}

use crate::wire::HeaderPair;

type HeaderTemplate = &'static [(&'static str, &'static str)];

#[derive(Debug)]
struct ShapeRow {
    fallback: Option<HeaderTemplate>,
    presets: &'static [(RequestKind, HeaderTemplate)],
    extends: Option<HeaderStyle>,
    append: HeaderTemplate,
    order: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone)]
enum Value {
    Literal(String),
    Field(Field),
    Template(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    UserAgent,
    SecChUa,
    AcceptLanguage,
    SecChUaMobile,
    FetchSite,
    NavigationSite,
    Origin,
    Referer,
    NavigationReferer,
}

impl Field {
    const ALL: [Self; 9] = [
        Self::UserAgent,
        Self::SecChUa,
        Self::AcceptLanguage,
        Self::SecChUaMobile,
        Self::FetchSite,
        Self::NavigationSite,
        Self::Origin,
        Self::Referer,
        Self::NavigationReferer,
    ];

    fn placeholder(self) -> &'static str {
        match self {
            Self::UserAgent => "{user_agent}",
            Self::SecChUa => "{sec_ch_ua}",
            Self::AcceptLanguage => "{accept_language}",
            Self::SecChUaMobile => "{sec_ch_ua_mobile}",
            Self::FetchSite => "{fetch_site}",
            Self::NavigationSite => "{navigation_site}",
            Self::Origin => "{origin}",
            Self::Referer => "{referer}",
            Self::NavigationReferer => "{navigation_referer}",
        }
    }
}

impl Value {
    fn of(raw: &str) -> Self {
        if let Some(field) = Field::ALL.into_iter().find(|f| f.placeholder() == raw) {
            return Self::Field(field);
        }
        if raw.contains('{') {
            Self::Template(raw.to_owned())
        } else {
            Self::Literal(raw.to_owned())
        }
    }

    fn reads(&self, field: Field) -> bool {
        match self {
            Self::Field(f) => *f == field,
            Self::Template(t) => t.contains(field.placeholder()),
            Self::Literal(_) => false,
        }
    }
}

type Compiled = Vec<(String, Value)>;

fn compile(template: HeaderTemplate) -> Compiled {
    template
        .iter()
        .map(|&(name, value)| (name.to_owned(), Value::of(value)))
        .collect()
}

#[derive(Debug)]
struct HeaderShape {
    fallback: Compiled,
    presets: HashMap<RequestKind, Compiled>,
    append: Vec<(String, String)>,
    order: Option<Vec<String>>,
}

const EXTRA_HEADERS: usize = 4;

static SHAPES: LazyLock<HashMap<HeaderStyle, HeaderShape>> = LazyLock::new(|| {
    SHAPE_ROWS
        .iter()
        .map(|&(style, _)| (style, resolve_shape(style)))
        .collect()
});

fn resolve_shape(style: HeaderStyle) -> HeaderShape {
    let (_, row) = SHAPE_ROWS
        .iter()
        .find(|(known, _)| *known == style)
        .expect("build.rs checks every extends target");
    let base = row.extends.map(resolve_shape);
    let mut presets = base
        .as_ref()
        .map(|base| base.presets.clone())
        .unwrap_or_default();
    presets.extend(
        row.presets
            .iter()
            .map(|&(kind, template)| (kind, compile(template))),
    );
    let fallback = row
        .fallback
        .map(compile)
        .or_else(|| base.map(|base| base.fallback))
        .expect("build.rs checks every shape reaches a fallback");
    HeaderShape {
        fallback,
        presets,
        append: row
            .append
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        order: row
            .order
            .map(|names| names.iter().map(|&name| name.to_owned()).collect()),
    }
}

impl<'a> HeaderContext<'a> {
    pub(crate) fn session(
        user_agent: &'a HeaderStr,
        sec_ch_ua: &'a HeaderStr,
        accept_language: &'a HeaderStr,
        platform: Platform,
    ) -> Self {
        let hints = !sec_ch_ua.is_empty();
        Self {
            user_agent,
            sec_ch_ua,
            sec_ch_ua_mobile: if hints { platform.mobile_flag() } else { "" },
            sec_ch_ua_platform: if hints {
                platform.sec_ch_platform()
            } else {
                ""
            },
            accept_language,
            origin: "",
            referer: "",
            fetch_site: "",
            navigation_site: "",
            navigation_referer: "",
        }
    }
}

impl HeaderContext<'_> {
    fn placeholders(&self) -> [(&'static str, &str); 10] {
        [
            (Field::UserAgent.placeholder(), self.user_agent.as_str()),
            (Field::SecChUa.placeholder(), self.sec_ch_ua.as_str()),
            (Field::SecChUaMobile.placeholder(), self.sec_ch_ua_mobile),
            ("{sec_ch_ua_platform}", self.sec_ch_ua_platform),
            (
                Field::AcceptLanguage.placeholder(),
                self.accept_language.as_str(),
            ),
            (Field::Origin.placeholder(), self.origin),
            (Field::Referer.placeholder(), self.referer),
            (Field::FetchSite.placeholder(), self.fetch_site),
            (Field::NavigationSite.placeholder(), self.navigation_site),
            (
                Field::NavigationReferer.placeholder(),
                self.navigation_referer,
            ),
        ]
    }

    fn expand(&self, value: &'static Value) -> Option<HeaderStr> {
        let shared = |v: &HeaderStr| (!v.is_empty()).then(|| v.clone());
        let fixed = |v: &'static str| (!v.is_empty()).then(|| HeaderStr::from_static(v));
        let copied = |v: &str| (!v.is_empty()).then(|| HeaderStr::from(v));
        match value {
            Value::Literal(literal) => Some(HeaderStr::from_static(literal)),
            Value::Field(Field::UserAgent) => shared(self.user_agent),
            Value::Field(Field::SecChUa) => shared(self.sec_ch_ua),
            Value::Field(Field::AcceptLanguage) => shared(self.accept_language),
            Value::Field(Field::SecChUaMobile) => fixed(self.sec_ch_ua_mobile),
            Value::Field(Field::FetchSite) => fixed(self.fetch_site),
            Value::Field(Field::NavigationSite) => fixed(self.navigation_site),
            Value::Field(Field::Origin) => copied(self.origin),
            Value::Field(Field::Referer) => copied(self.referer),
            Value::Field(Field::NavigationReferer) => copied(self.navigation_referer),
            Value::Template(template) => self.render(template),
        }
    }

    fn render(&self, template: &str) -> Option<HeaderStr> {
        let placeholders = self.placeholders();
        let mut value = String::with_capacity(template.len());
        let mut filled = false;
        let mut seen = false;
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            value.push_str(&rest[..start]);
            rest = &rest[start..];
            match placeholders.iter().find(|(key, _)| rest.starts_with(key)) {
                Some((key, replacement)) => {
                    seen = true;
                    filled |= !replacement.is_empty();
                    value.reserve(replacement.len() + rest.len());
                    value.push_str(replacement);
                    rest = &rest[key.len()..];
                }
                None => {
                    value.push('{');
                    rest = &rest[1..];
                }
            }
        }
        value.push_str(rest);
        (filled || !seen).then(|| HeaderStr::from(value))
    }
}

impl HeaderStyle {
    fn shape(self) -> &'static HeaderShape {
        SHAPES
            .get(&self)
            .expect("built-in header table covers every header style")
    }
}

#[derive(Debug, Clone)]
enum Slot {
    Fixed(HeaderStr),
    Request(&'static Value),
}

#[derive(Debug, Clone)]
struct Entry {
    name: &'static str,
    slot: Slot,
    body: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Plan {
    entries: Vec<Entry>,
    origin: bool,
    site: bool,
    referer: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct HeaderPlan {
    fallback: Plan,
    presets: Vec<(RequestKind, Plan)>,
    append: &'static [(String, String)],
    order: Option<&'static [String]>,
}

const PLAN_CACHE: NonZeroUsize = match NonZeroUsize::new(8) {
    Some(cap) => cap,
    None => NonZeroUsize::MIN,
};
type PlanCache = Mutex<LruCache<PlanKey, Arc<HeaderPlan>>>;

/// How a plan fills `Accept-Encoding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AcceptEncoding {
    Template,
    Fixed(&'static str),
    Omit,
}

impl AcceptEncoding {
    /// Bare plans send the session's codec list, or no field without one; others keep the template.
    pub(crate) fn of(style: HeaderStyle, codecs: Option<&'static str>) -> Self {
        match (style, codecs) {
            (HeaderStyle::Bare, Some(list)) => Self::Fixed(list),
            (HeaderStyle::Bare, None) => Self::Omit,
            _ => Self::Template,
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
struct PlanKey {
    style: HeaderStyle,
    encoding: AcceptEncoding,
    user_agent: String,
    sec_ch_ua: String,
    accept_language: String,
    sec_ch_ua_mobile: &'static str,
    sec_ch_ua_platform: String,
}

impl HeaderPlan {
    /// A plan shared by sessions with the same identity.
    pub(crate) fn shared(
        style: HeaderStyle,
        session: &HeaderContext<'_>,
        codecs: Option<&'static str>,
    ) -> Arc<Self> {
        static CACHE: LazyLock<PlanCache> = LazyLock::new(|| Mutex::new(LruCache::new(PLAN_CACHE)));
        Self::cached(&CACHE, style, session, AcceptEncoding::of(style, codecs))
    }

    fn cached(
        cache: &PlanCache,
        style: HeaderStyle,
        session: &HeaderContext<'_>,
        encoding: AcceptEncoding,
    ) -> Arc<Self> {
        let key = PlanKey {
            style,
            encoding,
            user_agent: session.user_agent.as_str().to_owned(),
            sec_ch_ua: session.sec_ch_ua.as_str().to_owned(),
            accept_language: session.accept_language.as_str().to_owned(),
            sec_ch_ua_mobile: session.sec_ch_ua_mobile,
            sec_ch_ua_platform: session.sec_ch_ua_platform.to_owned(),
        };
        let mut cache = cache.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(plan) = cache.get(&key) {
            return Arc::clone(plan);
        }
        let plan = Arc::new(Self::new(style, session, encoding));
        cache.put(key, Arc::clone(&plan));
        plan
    }

    pub(crate) fn new(
        style: HeaderStyle,
        session: &HeaderContext<'_>,
        encoding: AcceptEncoding,
    ) -> Self {
        let shape = style.shape();
        let plan = |template: &'static Compiled| Plan::new(template, session, encoding);
        Self {
            fallback: plan(&shape.fallback),
            presets: shape
                .presets
                .iter()
                .map(|(&kind, template)| (kind, plan(template)))
                .collect(),
            append: &shape.append,
            order: shape.order.as_deref(),
        }
    }

    pub(crate) fn plan(&self, kind: Option<RequestKind>) -> &Plan {
        kind.and_then(|kind| self.presets.iter().find(|(k, _)| *k == kind))
            .map_or(&self.fallback, |(_, plan)| plan)
    }

    pub(crate) fn append(&self) -> &'static [(String, String)] {
        self.append
    }

    pub(crate) fn order(&self) -> Option<&'static [String]> {
        self.order
    }
}

impl Plan {
    fn new(
        template: &'static Compiled,
        session: &HeaderContext<'_>,
        encoding: AcceptEncoding,
    ) -> Self {
        let mut plan = Self::default();
        for (name, value) in template {
            let site = value.reads(Field::FetchSite) || value.reads(Field::NavigationSite);
            let referer = value.reads(Field::Referer) || value.reads(Field::NavigationReferer);
            let encoding = if name.eq_ignore_ascii_case("accept-encoding") {
                encoding
            } else {
                AcceptEncoding::Template
            };
            let slot = match encoding {
                AcceptEncoding::Fixed(list) => Slot::Fixed(HeaderStr::from_static(list)),
                AcceptEncoding::Omit => continue,
                AcceptEncoding::Template if site || referer || value.reads(Field::Origin) => {
                    Slot::Request(value)
                }
                AcceptEncoding::Template => match session.expand(value) {
                    Some(fixed) => Slot::Fixed(fixed),
                    None => continue,
                },
            };
            plan.site |= site;
            plan.referer |= referer;
            plan.origin |= name.eq_ignore_ascii_case("origin");
            plan.entries.push(Entry {
                name,
                slot,
                body: is_body_header(name),
            });
        }
        plan
    }

    pub(crate) fn headers(&self, ctx: &HeaderContext<'_>, body: bool) -> Vec<HeaderPair> {
        let mut headers = Vec::with_capacity(self.entries.len() + EXTRA_HEADERS);
        for entry in &self.entries {
            if entry.body && !body {
                continue;
            }
            let value = match entry.slot {
                Slot::Fixed(ref fixed) => fixed.clone(),
                Slot::Request(value) => match ctx.expand(value) {
                    Some(value) => value,
                    None => continue,
                },
            };
            headers.push((Cow::Borrowed(entry.name), value));
        }
        headers
    }

    pub(crate) fn has_origin(&self) -> bool {
        self.origin
    }

    pub(crate) fn reads_site(&self) -> bool {
        self.site
    }

    pub(crate) fn reads_referer(&self) -> bool {
        self.referer
    }
}

#[cfg(test)]
mod tests;
