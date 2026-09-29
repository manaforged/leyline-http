use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/header_style.rs"));

mod kind;
pub use kind::Preset;

pub struct HeaderContext<'a> {
    pub user_agent: &'a str,
    pub sec_ch_ua: &'a str,
    pub sec_ch_ua_mobile: &'a str,
    pub sec_ch_ua_platform: &'a str,
    pub accept_language: &'a str,
    pub origin: &'a str,
    pub referer: &'a str,
    pub fetch_site: &'a str,
}

pub type HeaderPair = (Cow<'static, str>, Cow<'static, str>);

type HeaderTemplate = Vec<(String, String)>;

#[derive(Debug, Deserialize)]
struct ShapeRow {
    fallback: Option<HeaderTemplate>,
    #[serde(default)]
    presets: HashMap<Preset, HeaderTemplate>,
    extends: Option<HeaderStyle>,
    #[serde(default)]
    append: HeaderTemplate,
    order: Option<Vec<String>>,
}

#[derive(Debug)]
struct HeaderShape {
    fallback: HeaderTemplate,
    presets: HashMap<Preset, HeaderTemplate>,
    append: HeaderTemplate,
    order: Option<Vec<String>>,
}

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
    fn placeholders(&self) -> [(&'static str, &str); 8] {
        [
            ("{user_agent}", self.user_agent),
            ("{sec_ch_ua}", self.sec_ch_ua),
            ("{sec_ch_ua_mobile}", self.sec_ch_ua_mobile),
            ("{sec_ch_ua_platform}", self.sec_ch_ua_platform),
            ("{accept_language}", self.accept_language),
            ("{origin}", self.origin),
            ("{referer}", self.referer),
            ("{fetch_site}", self.fetch_site),
        ]
    }

    fn expand(&self, template: &'static str) -> Cow<'static, str> {
        if !template.contains('{') {
            return Cow::Borrowed(template);
        }
        let placeholders = self.placeholders();
        let mut value = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            value.push_str(&rest[..start]);
            rest = &rest[start..];
            match placeholders.iter().find(|(key, _)| rest.starts_with(key)) {
                Some((key, replacement)) => {
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
        Cow::Owned(value)
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
        preset: Option<Preset>,
        ctx: &HeaderContext<'_>,
    ) -> Vec<HeaderPair> {
        let shape = self.shape();
        let template = preset
            .and_then(|preset| shape.presets.get(&preset))
            .unwrap_or(&shape.fallback);
        template
            .iter()
            .map(|(name, value)| (Cow::Borrowed(name.as_str()), ctx.expand(value.as_str())))
            .collect()
    }
}

#[cfg(test)]
mod tests;
