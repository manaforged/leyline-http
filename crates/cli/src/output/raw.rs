//! Raw output — body bytes to stdout and nothing else.
//!
//! This is the default when stdout is piped, which makes
//! `leyline get url | jq`, `leyline get url | shasum`, and
//! `leyline get url > file.bin` all behave the way a Unix user
//! expects.

use std::io::Write;

use anyhow::{Context, Result};
use leyline::Response;

use crate::args::RequestArgs;

pub fn render(resp: &Response, args: &RequestArgs) -> Result<()> {
    if args.audit_only {
        // Raw + audit-only: print the fingerprint one-liner to stdout
        // as a single JSON object. The goal is that
        // `leyline get url --audit-only` is cleanly pipeable.
        if let Some(audit) = resp.audit() {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{}", audit.ja4).context("writing audit-only output")?;
        }
        return Ok(());
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(resp.bytes()).context("writing body")?;
    Ok(())
}
