use std::error::Error;
use std::path::PathBuf;

use crate::config::Config;

pub(crate) const OUT_BINDINGS_FILE: &str = "bindings.rs";

#[cfg(not(feature = "bindgen"))]
const COMMITTED_DIR: &str = "bindings";
#[cfg(not(feature = "bindgen"))]
const BINDGEN_FEATURE: &str = "bindgen";
#[cfg(not(feature = "bindgen"))]
const REGEN_SCRIPT: &str = "scripts/regen-bssl-bindings.sh";
#[cfg(not(feature = "bindgen"))]
const INCLUDE_PATH_VAR: &str = "LEYLINE_BSSL_INCLUDE_PATH";

#[cfg(feature = "bindgen")]
pub(crate) fn provide_bindings(config: &Config) -> Result<PathBuf, Box<dyn Error>> {
    crate::bindings::generate_bindings(config)
}

#[cfg(not(feature = "bindgen"))]
pub(crate) fn provide_bindings(config: &Config) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(include) = &config.env.include_path {
        return Err(format!(
            "{INCLUDE_PATH_VAR} is set to {}, but the committed bindings match the \
            headers that ship in leyline-bssl-sys. Enable the `{BINDGEN_FEATURE}` \
            feature of leyline-bssl-sys to generate bindings from those headers",
            include.display(),
        )
        .into());
    }
    let committed = config
        .manifest_dir
        .join(COMMITTED_DIR)
        .join(format!("{}.rs", config.target));
    println!("cargo:rerun-if-changed={}", committed.display());
    if !committed.is_file() {
        return Err(format!(
            "leyline-bssl-sys has no pre-generated bindings for target `{target}` \
            ({path} is missing). Enable the `{BINDGEN_FEATURE}` feature of \
            leyline-bssl-sys to generate them at build time, or run \
            `{REGEN_SCRIPT} {target}` in the leyline-http repository to add them",
            target = config.target,
            path = committed.display(),
        )
        .into());
    }
    let out = config.out_dir.join(OUT_BINDINGS_FILE);
    std::fs::copy(&committed, &out).map_err(|e| {
        format!(
            "could not copy {} to {}: {e}",
            committed.display(),
            out.display()
        )
    })?;
    Ok(out)
}
