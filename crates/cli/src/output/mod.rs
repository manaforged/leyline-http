//! Response renderers. One module per output format.

pub mod audit;
pub mod json;
pub mod pretty;
pub mod raw;

use anyhow::Result;
use leyline::Response;

use crate::args::{OutputFormat, RequestArgs};

/// Render a response using the given format. `method` and `url` are
/// passed explicitly because the pretty renderer wants to print the
/// original request line above the status.
pub fn render(
    format: OutputFormat,
    method: &str,
    url: &str,
    resp: &Response,
    args: &RequestArgs,
) -> Result<()> {
    match format {
        OutputFormat::Pretty => pretty::render(method, url, resp, args),
        OutputFormat::Json => json::render(method, url, resp, args),
        OutputFormat::Raw => raw::render(resp, args),
    }
}
