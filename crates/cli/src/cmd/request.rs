//! The single request handler that powers every HTTP verb subcommand
//! and the implicit-GET root form.

use anyhow::{Context, Result};

use crate::args::{OutputFormat, VerbArgs};
use crate::exit::ExitCode;
use crate::output;
use crate::session::{
    apply_request_args, build_session, effective_format, maybe_write_body_to_file,
};

pub async fn run(method: &str, mut args: VerbArgs) -> Result<ExitCode> {
    args.request.apply_verbose();

    let session = build_session(&args.request)?;
    let rb = session.request(method, &args.url);
    let rb = apply_request_args(rb, &args.request).await?;

    let resp = rb.send().await.context("request failed")?;

    let written_to_file = maybe_write_body_to_file(&resp, &args.request)?;

    let fmt = effective_format(&args.request);
    if written_to_file && matches!(fmt, OutputFormat::Raw) {
        // Body already on disk; nothing else to print in raw mode.
        return Ok(ExitCode::from_status(resp.status()));
    }
    output::render(fmt, method, resp.url(), &resp, &args.request)?;

    Ok(ExitCode::from_status(resp.status()))
}
