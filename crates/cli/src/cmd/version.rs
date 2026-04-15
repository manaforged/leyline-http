//! `leyline version` — CLI version + linked library + profile catalog.
//!
//! Kept separate from clap's built-in `--version` so we can include
//! the profile catalog and feature flags. `--version` still works
//! and prints just the crate version.

use std::io::Write;

use anyhow::Result;
use owo_colors::OwoColorize;

use leyline::ALL_BROWSERS;

use crate::args::browser_cli_name;
use crate::exit::ExitCode;

pub fn run() -> Result<ExitCode> {
    let mut out = anstream::stdout().lock();
    writeln!(
        out,
        "{} {}",
        "leyline-cli".bold(),
        env!("CARGO_PKG_VERSION").cyan()
    )?;
    writeln!(out, "{}", "──────────────".dimmed())?;
    writeln!(out, "features:")?;
    writeln!(out, "  http/1.1  ✓")?;
    writeln!(out, "  http/2    ✓")?;
    writeln!(out, "  http/3    ✓  (quiche + BoringSSL)")?;
    writeln!(out, "  websocket ✓")?;
    writeln!(out, "  proxy     ✓  (http, socks5)")?;
    writeln!(out)?;
    writeln!(out, "builtin profiles:")?;
    for b in ALL_BROWSERS {
        let (family, version) = b.profile_key();
        writeln!(
            out,
            "  {:<18}  {:<12} {}",
            browser_cli_name(b).cyan(),
            family,
            version
        )?;
    }
    Ok(ExitCode::Ok)
}
