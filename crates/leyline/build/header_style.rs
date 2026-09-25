use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use serde::Deserialize;

use crate::{BuildResult, is_ident};

#[derive(Deserialize)]
pub(crate) struct Shape {
    variant: String,
    #[serde(default)]
    default: bool,
    extends: Option<String>,
    fallback: Option<toml::Value>,
}

pub(crate) fn check_shapes(shapes: &BTreeMap<String, Shape>) -> BuildResult<()> {
    let mut variants = HashSet::new();
    for (key, shape) in shapes {
        if !is_ident(&shape.variant) || !variants.insert(shape.variant.as_str()) {
            return Err(
                format!("headers.toml: bad or repeated variant {:?}", shape.variant).into(),
            );
        }
        let mut seen = HashSet::from([key.as_str()]);
        let mut has_fallback = shape.fallback.is_some();
        let mut current = shape;
        while let Some(parent) = current.extends.as_deref() {
            if !seen.insert(parent) {
                return Err(format!("headers.toml: {key} has an extends cycle").into());
            }
            current = shapes
                .get(parent)
                .ok_or_else(|| format!("headers.toml: {key} extends unknown shape {parent:?}"))?;
            has_fallback |= current.fallback.is_some();
        }
        if !has_fallback {
            return Err(format!("headers.toml: {key} has no fallback in its extends chain").into());
        }
    }
    if shapes.values().filter(|s| s.default).count() != 1 {
        return Err("headers.toml: exactly one shape needs default = true".into());
    }
    Ok(())
}

pub(crate) fn render_shapes(shapes: &BTreeMap<String, Shape>) -> BuildResult<String> {
    let mut out = String::from(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]\n#[non_exhaustive]\npub enum HeaderStyle {\n",
    );
    for (key, shape) in shapes {
        if shape.default {
            writeln!(out, "    #[default]")?;
        }
        writeln!(
            out,
            "    #[serde(rename = {key:?})]\n    {},",
            shape.variant
        )?;
    }
    writeln!(out, "}}")?;
    Ok(out)
}
