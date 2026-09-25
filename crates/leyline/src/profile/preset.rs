use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum HeaderStyle {
    #[default]
    Chromium,
    Gecko,
    WebKit,
    OkHttp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Preset {
    Native,
    Navigate,
    Script,
    Xhr,
    Form,
    CrossOrigin,
    SameSite,
    FormNavigate,
}

pub struct HeaderContext<'a> {
    pub user_agent: &'a str,
    pub sec_ch_ua: &'a str,
    pub sec_ch_ua_mobile: &'a str,
    pub sec_ch_ua_platform: &'a str,
    pub accept_language: &'a str,
    pub origin: &'a str,
    pub referer: &'a str,
}

pub type HeaderPair = (Cow<'static, str>, Cow<'static, str>);

type HeaderTemplate = Vec<(String, String)>;

#[derive(Debug, Deserialize)]
struct HeaderShape {
    fallback: HeaderTemplate,
    #[serde(default)]
    presets: HashMap<Preset, HeaderTemplate>,
}

static SHAPES: LazyLock<HashMap<HeaderStyle, HeaderShape>> = LazyLock::new(|| {
    toml::from_str(include_str!("../../profiles/headers.toml"))
        .expect("built-in header table is statically valid")
});

impl HeaderContext<'_> {
    fn placeholders(&self) -> [(&'static str, &str); 7] {
        [
            ("{user_agent}", self.user_agent),
            ("{sec_ch_ua}", self.sec_ch_ua),
            ("{sec_ch_ua_mobile}", self.sec_ch_ua_mobile),
            ("{sec_ch_ua_platform}", self.sec_ch_ua_platform),
            ("{accept_language}", self.accept_language),
            ("{origin}", self.origin),
            ("{referer}", self.referer),
        ]
    }

    fn expand(&self, template: &'static str) -> Cow<'static, str> {
        if !template.contains('{') {
            return Cow::Borrowed(template);
        }
        let mut value = template.to_string();
        for (key, replacement) in self.placeholders() {
            value = value.replace(key, replacement);
        }
        Cow::Owned(value)
    }
}

impl HeaderStyle {
    pub(crate) fn build_headers(
        self,
        preset: Option<Preset>,
        ctx: &HeaderContext<'_>,
    ) -> Vec<HeaderPair> {
        let shape = SHAPES
            .get(&self)
            .expect("built-in header table covers every header style");
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
