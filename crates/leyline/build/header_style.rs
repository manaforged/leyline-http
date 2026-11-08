use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use serde::Deserialize;

use crate::preset_kind::RequestKind;
use crate::{BuildResult, is_ident, rust_option};

type Template = Vec<(String, String)>;

const PLACEHOLDERS: [&str; 10] = [
    "user_agent",
    "sec_ch_ua",
    "sec_ch_ua_mobile",
    "sec_ch_ua_platform",
    "accept_language",
    "origin",
    "referer",
    "fetch_site",
    "navigation_site",
    "navigation_referer",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Shape {
    id: u8,
    variant: String,
    #[serde(default)]
    default: bool,
    extends: Option<String>,
    fallback: Option<Template>,
    #[serde(default)]
    presets: HashMap<RequestKind, Template>,
    #[serde(default)]
    append: Template,
    order: Option<Vec<String>>,
}

fn check_names<'a>(key: &str, names: impl IntoIterator<Item = &'a str>) -> BuildResult<()> {
    let mut seen = HashSet::new();
    for name in names {
        let valid = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid || !seen.insert(name) {
            return Err(
                format!("headers.toml: {key} has a bad or repeated header {name:?}").into(),
            );
        }
    }
    Ok(())
}

fn check_placeholders(key: &str, value: &str) -> BuildResult<()> {
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        let tail = &rest[start + 1..];
        let end = tail.find('}').ok_or_else(|| {
            format!("headers.toml: {key} has an unclosed placeholder in {value:?}")
        })?;
        let name = &tail[..end];
        if !PLACEHOLDERS.contains(&name) {
            return Err(
                format!("headers.toml: {key} has an unknown placeholder {{{name}}}").into(),
            );
        }
        rest = &tail[end + 1..];
    }
    Ok(())
}

fn check_fields(key: &str, shape: &Shape) -> BuildResult<()> {
    let templates = shape
        .fallback
        .iter()
        .chain(shape.presets.values())
        .chain(std::iter::once(&shape.append));
    for template in templates {
        check_names(key, template.iter().map(|(name, _)| name.as_str()))?;
        for (_, value) in template {
            check_placeholders(key, value)?;
        }
    }
    check_names(key, shape.order.iter().flatten().map(String::as_str))
}

pub(crate) fn check_shapes(shapes: &BTreeMap<String, Shape>) -> BuildResult<()> {
    let mut variants = HashSet::new();
    let mut ids = HashSet::new();
    for (key, shape) in shapes {
        if !ids.insert(shape.id) {
            return Err(format!("headers.toml: {key} repeats id {}", shape.id).into());
        }
        if !is_ident(&shape.variant) || !variants.insert(shape.variant.as_str()) {
            return Err(
                format!("headers.toml: bad or repeated variant {:?}", shape.variant).into(),
            );
        }
        check_fields(key, shape)?;
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
    let mut ordered: Vec<_> = shapes.iter().collect();
    ordered.sort_by_key(|(_, shape)| shape.id);
    for (key, shape) in ordered {
        if shape.default {
            writeln!(out, "    #[default]")?;
        }
        writeln!(
            out,
            "    #[serde(rename = {key:?})]\n    {} = {},",
            shape.variant, shape.id
        )?;
    }
    writeln!(out, "}}\n")?;
    render_rows(&mut out, shapes)?;
    Ok(out)
}

pub(crate) fn style_variant<'a>(
    shapes: &'a BTreeMap<String, Shape>,
    key: &str,
) -> BuildResult<&'a str> {
    shapes
        .get(key)
        .map(|shape| shape.variant.as_str())
        .ok_or_else(|| format!("header_style {key:?} is not in headers.toml").into())
}

fn render_template(template: &[(String, String)]) -> String {
    let pairs: Vec<String> = template
        .iter()
        .map(|(name, value)| format!("({name:?}, {value:?})"))
        .collect();
    format!("&[{}]", pairs.join(", "))
}

fn render_row(shapes: &BTreeMap<String, Shape>, shape: &Shape) -> BuildResult<String> {
    let mut presets: Vec<_> = shape.presets.iter().collect();
    presets.sort_by_key(|(kind, _)| **kind as usize);
    let presets: Vec<String> = presets
        .into_iter()
        .map(|(kind, template)| format!("(RequestKind::{kind:?}, {})", render_template(template)))
        .collect();
    let extends = match shape.extends.as_deref() {
        Some(parent) => Some(format!("HeaderStyle::{}", style_variant(shapes, parent)?)),
        None => None,
    };
    let order = shape.order.as_ref().map(|names| format!("&{names:?}"));
    Ok(format!(
        "    (HeaderStyle::{}, ShapeRow {{ fallback: {}, presets: &[{}], extends: {}, append: {}, order: {} }}),",
        shape.variant,
        rust_option(shape.fallback.as_deref().map(render_template)),
        presets.join(", "),
        rust_option(extends),
        render_template(&shape.append),
        rust_option(order),
    ))
}

fn render_rows(out: &mut String, shapes: &BTreeMap<String, Shape>) -> BuildResult<()> {
    let mut ordered: Vec<&Shape> = shapes.values().collect();
    ordered.sort_by_key(|shape| shape.id);
    writeln!(out, "static SHAPE_ROWS: &[(HeaderStyle, ShapeRow)] = &[")?;
    for shape in ordered {
        writeln!(out, "{}", render_row(shapes, shape)?)?;
    }
    writeln!(out, "];")?;
    Ok(())
}
