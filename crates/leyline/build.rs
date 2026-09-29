use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

#[path = "build/brand.rs"]
mod brand;
#[path = "build/browser.rs"]
mod browser;
#[path = "build/header_style.rs"]
mod header_style;
#[path = "src/profile/preset/kind.rs"]
mod preset_kind;

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn is_ident(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn main() -> BuildResult<()> {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("profiles");
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src/profile/preset/kind.rs");
    let families: browser::Families =
        toml::from_str(&fs::read_to_string(root.join("families.toml"))?)?;
    let rows = browser::load_rows(&root, &families)?;
    browser::check_unique(&rows, &families)?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    fs::write(out.join("browser.rs"), browser::render(&rows, &families)?)?;
    let shapes: BTreeMap<String, header_style::Shape> =
        toml::from_str(&fs::read_to_string(root.join("headers.toml"))?)?;
    header_style::check_shapes(&shapes)?;
    for row in &rows {
        if let Some(style) = row
            .meta
            .header_style
            .as_deref()
            .filter(|s| !shapes.contains_key(*s))
        {
            return Err(format!(
                "{}: header_style {style:?} is not in headers.toml",
                row.path
            )
            .into());
        }
    }
    fs::write(
        out.join("header_style.rs"),
        header_style::render_shapes(&shapes)?,
    )?;
    let brands: BTreeMap<String, brand::Brand> =
        toml::from_str(&fs::read_to_string(root.join("brands.toml"))?)?;
    brand::check_brands(&brands, &shapes)?;
    fs::write(
        out.join("chromium_brand.rs"),
        brand::render_brands(&brands)?,
    )?;
    Ok(())
}
