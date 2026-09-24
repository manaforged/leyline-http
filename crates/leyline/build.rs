use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

type BuildResult<T> = Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Families {
    family: Vec<FamilyRow>,
}

#[derive(Deserialize)]
struct FamilyRow {
    variant: String,
    label: String,
    browsers: Vec<String>,
    #[serde(default)]
    default: bool,
}

#[derive(Deserialize)]
struct ProfileFile {
    meta: Meta,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Capture {
    Browser,
    HeadlessShell,
    Webview,
    Inferred,
    SelfReferential,
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    browser: String,
    version: u32,
    capture: Capture,
    variant: String,
    #[serde(default)]
    hello: Option<u32>,
    #[serde(default)]
    platform_browser: BTreeMap<String, String>,
    #[serde(default)]
    deprecated: Option<String>,
}

struct Row {
    meta: Meta,
    path: String,
    family: usize,
}

fn main() -> BuildResult<()> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("profiles");
    println!("cargo:rerun-if-changed={}", root.display());
    let families: Families = toml::from_str(&fs::read_to_string(root.join("families.toml"))?)?;
    let rows = load_rows(&root, &families)?;
    check_unique(&rows, &families)?;
    let code = render(&rows, &families)?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?).join("browser.rs");
    fs::write(out, code)?;
    Ok(())
}

fn load_rows(root: &Path, families: &Families) -> BuildResult<Vec<Row>> {
    let mut rows = Vec::new();
    for dir in sorted(root)? {
        if !dir.is_dir() {
            continue;
        }
        for file in sorted(&dir)? {
            if file.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let text = fs::read_to_string(&file)?;
            let meta = toml::from_str::<ProfileFile>(&text)
                .map_err(|e| format!("{}: {e}", file.display()))?
                .meta;
            let family = families
                .family
                .iter()
                .position(|f| f.browsers.contains(&meta.browser))
                .ok_or_else(|| {
                    format!(
                        "{}: browser {:?} is in no families.toml entry",
                        file.display(),
                        meta.browser
                    )
                })?;
            let path = file
                .strip_prefix(root)?
                .to_str()
                .ok_or("profile path is not UTF-8")?
                .replace('\\', "/");
            rows.push(Row { meta, path, family });
        }
    }
    rows.sort_by(|a, b| {
        (a.family, &a.meta.browser, a.meta.version).cmp(&(
            b.family,
            &b.meta.browser,
            b.meta.version,
        ))
    });
    Ok(rows)
}

fn sorted(dir: &Path) -> BuildResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        paths.push(entry?.path());
    }
    paths.sort();
    Ok(paths)
}

fn is_ident(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn check_unique(rows: &[Row], families: &Families) -> BuildResult<()> {
    let mut variants = HashSet::new();
    let mut keys = HashSet::new();
    for row in rows {
        if !is_ident(&row.meta.variant) {
            return Err(format!(
                "{}: variant {:?} is not a Rust type name",
                row.path, row.meta.variant
            )
            .into());
        }
        if !variants.insert(row.meta.variant.as_str()) {
            return Err(format!(
                "{}: variant {} is declared twice",
                row.path, row.meta.variant
            )
            .into());
        }
        if !keys.insert((row.meta.browser.as_str(), row.meta.version)) {
            return Err(format!(
                "{}: {} {} is declared twice",
                row.path, row.meta.browser, row.meta.version
            )
            .into());
        }
    }
    let mut family_names = HashSet::new();
    for family in &families.family {
        if !is_ident(&family.variant) || !family_names.insert(family.variant.as_str()) {
            return Err(format!(
                "families.toml: bad or repeated variant {:?}",
                family.variant
            )
            .into());
        }
    }
    if families.family.iter().filter(|f| f.default).count() != 1 {
        return Err("families.toml: exactly one family needs default = true".into());
    }
    Ok(())
}

fn find<'a>(rows: &'a [Row], browser: &str, version: u32) -> BuildResult<&'a Row> {
    rows.iter()
        .find(|r| r.meta.browser == browser && r.meta.version == version)
        .ok_or_else(|| format!("no profile for {browser} {version}").into())
}

fn latest(rows: &[Row], keep: impl Fn(&Row) -> bool) -> Option<&Row> {
    rows.iter()
        .filter(|r| r.meta.deprecated.is_none() && keep(r))
        .max_by_key(|r| r.meta.version)
}

fn hello<'a>(rows: &'a [Row], row: &Row) -> BuildResult<&'a Row> {
    find(
        rows,
        &row.meta.browser,
        row.meta.hello.unwrap_or(row.meta.version),
    )
}

fn variant_list<'a>(rows: impl IntoIterator<Item = &'a Row>) -> String {
    rows.into_iter()
        .map(|r| format!("Browser::{}", r.meta.variant))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render(rows: &[Row], families: &Families) -> BuildResult<String> {
    let mut out = String::new();
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n#[non_exhaustive]\npub enum Family {{"
    )?;
    for family in &families.family {
        writeln!(out, "    {},", family.variant)?;
    }
    writeln!(out, "}}\n")?;
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n#[non_exhaustive]\npub enum Browser {{"
    )?;
    for row in rows {
        if let Some(note) = &row.meta.deprecated {
            writeln!(out, "    #[deprecated(note = {note:?})]")?;
        }
        writeln!(out, "    {},", row.meta.variant)?;
    }
    writeln!(out, "}}\n")?;

    let labels: Vec<String> = families
        .family
        .iter()
        .map(|f| format!("{:?}", f.label))
        .collect();
    writeln!(
        out,
        "const FAMILY_LABELS: &[&str] = &[{}];",
        labels.join(", ")
    )?;
    let mut family_latest = Vec::new();
    for (index, family) in families.family.iter().enumerate() {
        let row = latest(rows, |r| {
            r.family == index && r.meta.capture == Capture::Browser
        })
        .or_else(|| latest(rows, |r| r.family == index))
        .ok_or_else(|| format!("families.toml: family {} has no profile", family.variant))?;
        family_latest.push(row);
    }
    writeln!(
        out,
        "#[allow(deprecated)]\nconst FAMILY_LATEST: &[Browser] = &[{}];",
        variant_list(family_latest.iter().copied())
    )?;
    let default = families
        .family
        .iter()
        .find(|f| f.default)
        .ok_or("no default family")?;
    writeln!(
        out,
        "const DEFAULT_FAMILY: Family = Family::{};",
        default.variant
    )?;
    writeln!(
        out,
        "#[allow(deprecated)]\nconst ALL: &[Browser] = &[{}];",
        variant_list(rows)
    )?;

    writeln!(out, "#[allow(deprecated)]\nconst ENTRIES: &[Entry] = &[")?;
    for row in rows {
        let rep = hello(rows, row)?;
        let mut platforms = Vec::new();
        for (platform, browser) in &row.meta.platform_browser {
            let target = latest(rows, |r| &r.meta.browser == browser).ok_or_else(|| {
                format!(
                    "{}: platform_browser names unknown browser {browser:?}",
                    row.path
                )
            })?;
            platforms.push(format!("({platform:?}, Browser::{})", target.meta.variant));
        }
        writeln!(
            out,
            "    Entry {{ family: Family::{}, key: {:?}, version: {}, name: {:?}, hello: Browser::{}, \
             platforms: &[{}], \
             source: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/profiles/{}\")) }},",
            families.family[row.family].variant,
            row.meta.browser,
            row.meta.version,
            row.meta.name,
            rep.meta.variant,
            platforms.join(", "),
            row.path,
        )?;
    }
    writeln!(out, "];")?;
    Ok(out)
}
