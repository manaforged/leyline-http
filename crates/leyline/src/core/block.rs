use std::sync::{Arc, LazyLock};

use http::{HeaderName, StatusCode};
use serde::Deserialize;

use crate::core::error::{Cause, Error, Result};
use crate::core::response::Response;

const STATUS_VENDOR: &str = "status";
const BUILTIN: &str = include_str!("../../profiles/blocks.toml");

static BUILTIN_RULES: LazyLock<Rules> =
    LazyLock::new(|| Rules::from_toml(BUILTIN).expect("bundled block rules are statically valid"));

/// A bot-protection page detected in a response, with the vendor name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Block {
    /// The vendor served a challenge page.
    Challenge(String),
    /// The vendor served a captcha.
    Captcha(String),
    /// The vendor denied the request.
    Denied(String),
}

impl Block {
    /// Returns the name of the vendor that served the block.
    pub fn vendor(&self) -> &str {
        match self {
            Block::Challenge(vendor) | Block::Captcha(vendor) | Block::Denied(vendor) => vendor,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Challenge,
    Captcha,
    #[serde(alias = "block")]
    Denied,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Rules {
    rules: Arc<Vec<BlockRule>>,
}

#[derive(Debug, Clone)]
struct BlockRule {
    vendor: String,
    kind: Kind,
    status: Option<StatusCode>,
    header: Option<HeaderName>,
    value: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    #[serde(default)]
    rule: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleEntry {
    vendor: String,
    kind: Kind,
    status: Option<u16>,
    header: Option<String>,
    value: Option<String>,
}

impl Rules {
    pub(crate) fn builtin() -> Self {
        BUILTIN_RULES.clone()
    }

    pub(crate) fn from_toml(source: &str) -> Result<Self> {
        let file: RulesFile =
            toml::from_str(source).map_err(|e| Error::new(Cause::Config).with_source(e))?;
        let rules = file
            .rule
            .into_iter()
            .map(BlockRule::try_from)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            rules: Arc::new(rules),
        })
    }

    pub(crate) fn statuses(statuses: impl IntoIterator<Item = u16>) -> Self {
        let rules = statuses
            .into_iter()
            .filter_map(|code| StatusCode::from_u16(code).ok())
            .map(|status| BlockRule {
                vendor: STATUS_VENDOR.to_string(),
                kind: Kind::Denied,
                status: Some(status),
                header: None,
                value: None,
            })
            .collect();
        Self {
            rules: Arc::new(rules),
        }
    }

    pub(crate) fn extend(&mut self, other: Rules) {
        Arc::make_mut(&mut self.rules).extend(other.rules.iter().cloned());
    }

    pub(crate) fn check(&self, response: &Response) -> Option<Block> {
        self.rules
            .iter()
            .find(|rule| rule.matches(response))
            .map(BlockRule::block)
    }
}

impl TryFrom<RuleEntry> for BlockRule {
    type Error = Error;

    fn try_from(entry: RuleEntry) -> Result<Self> {
        let invalid = |message: String| Error::new(Cause::Config).with_message(message);
        if entry.status.is_none() && entry.header.is_none() {
            return Err(invalid(format!(
                "block rule for {} needs a status or a header",
                entry.vendor
            )));
        }
        if entry.value.is_some() && entry.header.is_none() {
            return Err(invalid(format!(
                "block rule for {} has a value without a header",
                entry.vendor
            )));
        }
        let status = entry
            .status
            .map(StatusCode::from_u16)
            .transpose()
            .map_err(|e| Error::new(Cause::Config).with_source(e))?;
        let header = entry
            .header
            .map(|name| HeaderName::from_bytes(name.as_bytes()))
            .transpose()
            .map_err(|e| Error::new(Cause::Config).with_source(e))?;
        Ok(Self {
            vendor: entry.vendor,
            kind: entry.kind,
            status,
            header,
            value: entry.value,
        })
    }
}

impl BlockRule {
    fn block(&self) -> Block {
        let vendor = self.vendor.clone();
        match self.kind {
            Kind::Challenge => Block::Challenge(vendor),
            Kind::Captcha => Block::Captcha(vendor),
            Kind::Denied => Block::Denied(vendor),
        }
    }

    fn matches(&self, response: &Response) -> bool {
        if self
            .status
            .is_some_and(|status| status != response.status())
        {
            return false;
        }
        let Some(header) = &self.header else {
            return true;
        };
        let mut values = response.headers().get_all(header).iter().peekable();
        let Some(expected) = &self.value else {
            return values.peek().is_some();
        };
        values
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.trim().eq_ignore_ascii_case(expected))
    }
}
