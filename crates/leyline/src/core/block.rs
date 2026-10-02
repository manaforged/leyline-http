use std::sync::{Arc, LazyLock};

use http::{HeaderName, StatusCode};
use serde::Deserialize;

use crate::core::error::{Error, Kind, Result};
use crate::core::response::Response;

const STATUS_VENDOR: &str = "status";
const BUILTIN: &str = include_str!("../../profiles/blocks.toml");

static BUILTIN_RULES: LazyLock<BlockRules> = LazyLock::new(|| {
    BlockRules::from_toml(BUILTIN).expect("bundled block rules are statically valid")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum BlockKind {
    Challenge,
    Captcha,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlockSignal {
    pub vendor: String,
    pub kind: BlockKind,
}

#[derive(Debug, Clone, Default)]
pub struct BlockRules {
    rules: Arc<Vec<BlockRule>>,
}

#[derive(Debug, Clone)]
struct BlockRule {
    vendor: String,
    kind: BlockKind,
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
    kind: BlockKind,
    status: Option<u16>,
    header: Option<String>,
    value: Option<String>,
}

impl BlockRules {
    pub fn builtin() -> &'static BlockRules {
        &BUILTIN_RULES
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        let file: RulesFile =
            toml::from_str(source).map_err(|e| Error::new(Kind::Config).with_source(e))?;
        let rules = file
            .rule
            .into_iter()
            .map(BlockRule::try_from)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            rules: Arc::new(rules),
        })
    }

    pub fn statuses(statuses: impl IntoIterator<Item = u16>) -> Self {
        let rules = statuses
            .into_iter()
            .filter_map(|code| StatusCode::from_u16(code).ok())
            .map(|status| BlockRule {
                vendor: STATUS_VENDOR.to_string(),
                kind: BlockKind::Block,
                status: Some(status),
                header: None,
                value: None,
            })
            .collect();
        Self {
            rules: Arc::new(rules),
        }
    }

    pub fn extend(&mut self, other: BlockRules) {
        Arc::make_mut(&mut self.rules).extend(other.rules.iter().cloned());
    }

    pub fn check(&self, response: &Response) -> Option<BlockSignal> {
        self.rules
            .iter()
            .find(|rule| rule.matches(response))
            .map(|rule| BlockSignal {
                vendor: rule.vendor.clone(),
                kind: rule.kind,
            })
    }
}

impl TryFrom<RuleEntry> for BlockRule {
    type Error = Error;

    fn try_from(entry: RuleEntry) -> Result<Self> {
        let invalid = |message: String| Error::new(Kind::Config).with_message(message);
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
            .map_err(|e| Error::new(Kind::Config).with_source(e))?;
        let header = entry
            .header
            .map(|name| HeaderName::from_bytes(name.as_bytes()))
            .transpose()
            .map_err(|e| Error::new(Kind::Config).with_source(e))?;
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
