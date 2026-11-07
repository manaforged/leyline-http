use std::borrow::Cow;
use std::sync::{Arc, LazyLock, Mutex};

use crate::header_str::HeaderStr;
use crate::profile::Platform;
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

pub type HeaderPair = (Cow<'static, str>, HeaderStr);

type HeaderTemplate = Vec<(String, String)>;

#[derive(Debug, Deserialize)]
struct ShapeRow {
    fallback: Option<HeaderTemplate>,
    #[serde(default)]
    presets: HashMap<RequestKind, HeaderTemplate>,
    extends: Option<HeaderStyle>,
    #[serde(default)]
    append: HeaderTemplate,
    order: Option<Vec<String>>,
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

fn compile(template: &HeaderTemplate) -> Compiled {
    template
        .iter()
        .map(|(name, value)| (name.clone(), Value::of(value)))
        .collect()
}

#[derive(Debug)]
struct HeaderShape {
    fallback: Compiled,
    presets: HashMap<RequestKind, Compiled>,
    append: HeaderTemplate,
    order: Option<Vec<String>>,
}

const EXTRA_HEADERS: usize = 4;

static SHAPES: LazyLock<HashMap<HeaderStyle, HeaderShape>> = LazyLock::new(|| {
    let rows: HashMap<HeaderStyle, ShapeRow> =
        toml::from_str(include_str!("../../profiles/headers.toml"))
            .expect("built-in header table is statically valid");
    rows.keys()
        .map(|&style| (style, resolve_shape(&rows, style)))
        .collect()
});

fn resolve_shape(rows: &HashMap<HeaderStyle, ShapeRow>, style: HeaderStyle) -> HeaderShape {
    let row = rows
        .get(&style)
        .expect("build.rs checks every extends target");
    let base = row.extends.map(|parent| resolve_shape(rows, parent));
    let mut presets = base
        .as_ref()
        .map(|base| base.presets.clone())
        .unwrap_or_default();
    presets.extend(
        row.presets
            .iter()
            .map(|(&kind, template)| (kind, compile(template))),
    );
    let fallback = row
        .fallback
        .as_ref()
        .map(compile)
        .or_else(|| base.map(|base| base.fallback))
        .expect("build.rs checks every shape reaches a fallback");
    HeaderShape {
        fallback,
        presets,
        append: row.append.clone(),
        order: row.order.clone(),
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

const BODY_HEADERS: [&str; 4] = [
    "content-encoding",
    "content-language",
    "content-location",
    "content-type",
];

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

const PLAN_CACHE: usize = 8;
type PlanCache = Mutex<Vec<(PlanKey, Arc<HeaderPlan>)>>;

#[derive(PartialEq, Eq)]
struct PlanKey {
    style: HeaderStyle,
    codecs: Option<&'static str>,
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
        static CACHE: PlanCache = Mutex::new(Vec::new());
        Self::cached(&CACHE, style, session, codecs)
    }

    fn cached(
        cache: &PlanCache,
        style: HeaderStyle,
        session: &HeaderContext<'_>,
        codecs: Option<&'static str>,
    ) -> Arc<Self> {
        let key = PlanKey {
            style,
            codecs,
            user_agent: session.user_agent.as_str().to_owned(),
            sec_ch_ua: session.sec_ch_ua.as_str().to_owned(),
            accept_language: session.accept_language.as_str().to_owned(),
            sec_ch_ua_mobile: session.sec_ch_ua_mobile,
            sec_ch_ua_platform: session.sec_ch_ua_platform.to_owned(),
        };
        let mut cache = cache.lock().unwrap_or_else(|err| err.into_inner());
        if let Some((_, plan)) = cache.iter().find(|(cached, _)| cached == &key) {
            return Arc::clone(plan);
        }
        let plan = Arc::new(Self::new(style, session, codecs));
        if cache.len() < PLAN_CACHE {
            cache.push((key, Arc::clone(&plan)));
        }
        plan
    }

    pub(crate) fn new(
        style: HeaderStyle,
        session: &HeaderContext<'_>,
        codecs: Option<&'static str>,
    ) -> Self {
        let shape = style.shape();
        let codecs = (style == HeaderStyle::Bare).then_some(codecs);
        let plan = |template: &'static Compiled| Plan::new(template, session, codecs);
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
        codecs: Option<Option<&'static str>>,
    ) -> Self {
        let mut plan = Self::default();
        for (name, value) in template {
            let site = value.reads(Field::FetchSite) || value.reads(Field::NavigationSite);
            let referer = value.reads(Field::Referer) || value.reads(Field::NavigationReferer);
            let slot = match codecs.filter(|_| name.eq_ignore_ascii_case("accept-encoding")) {
                Some(Some(list)) => Slot::Fixed(HeaderStr::from_static(list)),
                Some(None) => continue,
                None if site || referer || value.reads(Field::Origin) => Slot::Request(value),
                None => match session.expand(value) {
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
                body: BODY_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h)),
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
