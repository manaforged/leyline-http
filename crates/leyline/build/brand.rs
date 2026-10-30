use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use serde::Deserialize;

use crate::browser::Row;
use crate::header_style::Shape;
use crate::{BuildResult, is_ident};

#[derive(Deserialize)]
pub(crate) struct Brand {
    #[serde(default)]
    default: bool,
    header_style: Option<String>,
    ua_token: Option<String>,
    #[serde(default)]
    follow_chromium: bool,
    #[serde(default)]
    versions: BTreeMap<String, Vec<String>>,
}

pub(crate) struct Variant {
    pub(crate) name: String,
    pub(crate) brand: String,
    pub(crate) base: String,
    pub(crate) version: u32,
    pub(crate) deprecated: Option<String>,
    pub(crate) id: u32,
}

pub(crate) fn variants(
    brands: &BTreeMap<String, Brand>,
    rows: &[Row],
) -> BuildResult<Vec<Variant>> {
    let mut out = Vec::new();
    for (name, brand) in brands.iter().filter(|(_, b)| b.ua_token.is_some()) {
        for row in rows.iter().filter(|r| r.meta.browser == "chrome") {
            let mut majors: Vec<u32> = if brand.follow_chromium {
                vec![row.meta.version]
            } else {
                brand
                    .versions
                    .get(&row.meta.version.to_string())
                    .into_iter()
                    .flatten()
                    .map(|v| {
                        v.split('.')
                            .next()
                            .and_then(|m| m.parse().ok())
                            .ok_or_else(|| format!("brands.toml: {name}: bad version {v:?}"))
                    })
                    .collect::<Result<_, _>>()?
            };
            majors.dedup();
            for version in majors {
                out.push(Variant {
                    name: format!("{name}{version}"),
                    brand: name.clone(),
                    base: row.meta.variant.clone(),
                    version,
                    deprecated: row.meta.deprecated.clone(),
                    id: 0,
                });
            }
        }
    }
    Ok(out)
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
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]\npub(crate) enum ChromiumBrand {\n",
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
