use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "build/brand.rs"]
mod brand;
#[path = "build/browser.rs"]
mod browser;
#[path = "build/entities.rs"]
mod entities;
#[path = "build/header_style.rs"]
mod header_style;
#[path = "src/profile/preset/kind.rs"]
mod preset_kind;

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn is_ident(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

type Shapes = BTreeMap<String, header_style::Shape>;

fn main() -> BuildResult<()> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let root = manifest.join("profiles");
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src/profile/preset/kind.rs");
    let entity_data = manifest.join("data").join("entities.tsv");
    println!("cargo:rerun-if-changed={}", entity_data.display());
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let rows = write_browsers(&root, &out)?;
    let shapes = write_header_styles(&root, &out, &rows)?;
    write_brands(&root, &out, &shapes)?;
    write_entities(&entity_data, &out)
}

fn write_browsers(root: &Path, out: &Path) -> BuildResult<Vec<browser::Row>> {
    let families: browser::Families =
        toml::from_str(&fs::read_to_string(root.join("families.toml"))?)?;
    let rows = browser::load_rows(root, &families)?;
    browser::check_unique(&rows, &families)?;
    fs::write(out.join("browser.rs"), browser::render(&rows, &families)?)?;
    Ok(rows)
}

fn write_header_styles(root: &Path, out: &Path, rows: &[browser::Row]) -> BuildResult<Shapes> {
    let shapes: Shapes = toml::from_str(&fs::read_to_string(root.join("headers.toml"))?)?;
    header_style::check_shapes(&shapes)?;
    check_row_styles(rows, &shapes)?;
    fs::write(
        out.join("header_style.rs"),
        header_style::render_shapes(&shapes)?,
    )?;
    Ok(shapes)
}

fn check_row_styles(rows: &[browser::Row], shapes: &Shapes) -> BuildResult<()> {
    let missing = rows.iter().find_map(|row| {
        row.meta
            .header_style
            .as_deref()
            .filter(|s| !shapes.contains_key(*s))
            .map(|style| (row, style))
    });
    match missing {
        Some((row, style)) => Err(format!(
            "{}: header_style {style:?} is not in headers.toml",
            row.path
        )
        .into()),
        None => Ok(()),
    }
}

fn write_brands(root: &Path, out: &Path, shapes: &Shapes) -> BuildResult<()> {
    let brands: BTreeMap<String, brand::Brand> =
        toml::from_str(&fs::read_to_string(root.join("brands.toml"))?)?;
    brand::check_brands(&brands, shapes)?;
    fs::write(
        out.join("chromium_brand.rs"),
        brand::render_brands(&brands)?,
    )?;
    Ok(())
}

fn write_entities(data: &Path, out: &Path) -> BuildResult<()> {
    fs::write(
        out.join("entities.rs"),
        entities::render(&fs::read_to_string(data)?)?,
    )?;
    Ok(())
}
