use std::process::ExitCode;

use crate::archive::emit_link_directives;
use crate::bindings::generate_bindings;
use crate::config::Config;
use crate::source::ensure_patches_applied;

mod archive;
mod bindings;
mod cmake;
mod config;
mod prefix;
mod process;
mod source;
mod targets;

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("leyline-bssl-sys failed: {e}");
        println!(
            "cargo::error={}",
            e.to_string().trim_ascii().replace('\n', "\ncargo::error=")
        );
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    config.check_supported_target()?;
    ensure_patches_applied(&config)?;
    if !config.env.docs_rs {
        emit_link_directives(&config)?;
    }
    generate_bindings(&config).map_err(|e| format!("could not generate bindings: {e}"))?;
    Ok(())
}
