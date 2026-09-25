use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use serde::Deserialize;

use crate::header_style::Shape;
use crate::{BuildResult, is_ident};

#[derive(Deserialize)]
pub(crate) struct Brand {
    #[serde(default)]
    default: bool,
    header_style: Option<String>,
}

pub(crate) fn check_brands(
    brands: &BTreeMap<String, Brand>,
    shapes: &BTreeMap<String, Shape>,
) -> BuildResult<()> {
    let mut keys = HashSet::new();
    for (name, brand) in brands {
        if !is_ident(name) || !keys.insert(name.to_ascii_lowercase()) {
            return Err(format!("brands.toml: bad or repeated brand {name:?}").into());
        }
        if let Some(style) = brand
            .header_style
            .as_deref()
            .filter(|s| !shapes.contains_key(*s))
        {
            return Err(format!(
                "brands.toml: {name}: header_style {style:?} is not in headers.toml"
            )
            .into());
        }
    }
    if brands.values().filter(|b| b.default).count() != 1 {
        return Err("brands.toml: exactly one brand needs default = true".into());
    }
    Ok(())
}

pub(crate) fn render_brands(brands: &BTreeMap<String, Brand>) -> BuildResult<String> {
    let mut out = String::from(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]\n#[non_exhaustive]\npub enum ChromiumBrand {\n",
    );
    for (name, brand) in brands {
        if brand.default {
            writeln!(out, "    #[default]")?;
        }
        writeln!(out, "    {name},")?;
    }
    writeln!(out, "}}\n")?;
    let all: Vec<String> = brands
        .keys()
        .map(|n| format!("ChromiumBrand::{n}"))
        .collect();
    let labels: Vec<String> = brands.keys().map(|n| format!("{n:?}")).collect();
    let keys: Vec<String> = brands
        .keys()
        .map(|n| format!("{:?}", n.to_ascii_lowercase()))
        .collect();
    writeln!(
        out,
        "const BRAND_ALL: &[ChromiumBrand] = &[{}];",
        all.join(", ")
    )?;
    writeln!(
        out,
        "const BRAND_LABELS: &[&str] = &[{}];",
        labels.join(", ")
    )?;
    writeln!(out, "const BRAND_KEYS: &[&str] = &[{}];", keys.join(", "))?;
    Ok(out)
}
