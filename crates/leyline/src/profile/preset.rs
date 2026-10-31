use std::borrow::Cow;

use crate::header_str::HeaderStr;
use std::collections::HashMap;
use std::sync::LazyLock;

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

#[derive(Debug)]
struct HeaderShape {
    fallback: HeaderTemplate,
    presets: HashMap<RequestKind, HeaderTemplate>,
    append: HeaderTemplate,
    order: Option<Vec<String>>,
}

const EXTRA_HEADERS: usize = 8;

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
    presets.extend(row.presets.clone());
    let fallback = row
        .fallback
        .clone()
        .or_else(|| base.map(|base| base.fallback))
        .expect("build.rs checks every shape reaches a fallback");
    HeaderShape {
        fallback,
        presets,
        append: row.append.clone(),
        order: row.order.clone(),
    }
}

impl HeaderContext<'_> {
    fn placeholders(&self) -> [(&'static str, &str); 10] {
        [
            ("{user_agent}", self.user_agent.as_str()),
            ("{sec_ch_ua}", self.sec_ch_ua.as_str()),
            ("{sec_ch_ua_mobile}", self.sec_ch_ua_mobile),
            ("{sec_ch_ua_platform}", self.sec_ch_ua_platform),
            ("{accept_language}", self.accept_language.as_str()),
            ("{origin}", self.origin),
            ("{referer}", self.referer),
            ("{fetch_site}", self.fetch_site),
            ("{navigation_site}", self.navigation_site),
            ("{navigation_referer}", self.navigation_referer),
        ]
    }

    fn shared(&self) -> [(&'static str, &HeaderStr); 3] {
        [
            ("{user_agent}", self.user_agent),
            ("{sec_ch_ua}", self.sec_ch_ua),
            ("{accept_language}", self.accept_language),
        ]
    }

    fn statics(&self) -> [(&'static str, &'static str); 3] {
        [
            ("{sec_ch_ua_mobile}", self.sec_ch_ua_mobile),
            ("{fetch_site}", self.fetch_site),
            ("{navigation_site}", self.navigation_site),
        ]
    }

    fn expand(&self, template: &'static str) -> Option<HeaderStr> {
        if !template.contains('{') {
            return Some(HeaderStr::from_static(template));
        }
        if let Some((_, value)) = self.shared().into_iter().find(|(key, _)| *key == template) {
            return (!value.is_empty()).then(|| value.clone());
        }
        if let Some((_, value)) = self.statics().into_iter().find(|(key, _)| *key == template) {
            return (!value.is_empty()).then(|| HeaderStr::from_static(value));
        }
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

    pub(crate) fn append(self) -> &'static [(String, String)] {
        &self.shape().append
    }

    pub(crate) fn order(self) -> Option<&'static [String]> {
        self.shape().order.as_deref()
    }

    pub(crate) fn build_headers(
        self,
        preset: Option<RequestKind>,
        ctx: &HeaderContext<'_>,
    ) -> Vec<HeaderPair> {
        let shape = self.shape();
        let template = preset
            .and_then(|preset| shape.presets.get(&preset))
            .unwrap_or(&shape.fallback);
        let mut headers = Vec::with_capacity(template.len() + EXTRA_HEADERS);
        headers.extend(template.iter().filter_map(|(name, value)| {
            ctx.expand(value.as_str())
                .map(|rendered| (Cow::Borrowed(name.as_str()), rendered))
        }));
        headers
    }
}

#[cfg(test)]
mod tests;
