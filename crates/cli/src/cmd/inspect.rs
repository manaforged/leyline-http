//! `leyline inspect <url>` — HEAD request with verbose audit output.
//!
//! A thin wrapper around `cmd::request::run("HEAD", _)` with `-v`
//! forced on.
//!
//! HEAD has no body, so `pretty::write_body` naturally prints
//! nothing — there's no reason to also set `audit_only`, which would
//! (correctly) suppress the `── wire ──` block that the whole
//! fingerprint-debugging workflow is here to see.

use anyhow::Result;

use crate::args::VerbArgs;
use crate::cmd::request;
use crate::exit::ExitCode;

pub async fn run(mut args: VerbArgs) -> Result<ExitCode> {
    args.request.verbose = true;
    request::run("HEAD", args).await
}
