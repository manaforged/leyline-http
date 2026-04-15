//! Pretty TTY renderer. httpie-inspired; hand-rolled JSON colorizer
//! for body highlighting (no syntect, no 5 MB of grammar files).

use std::io::Write;

use anyhow::{Context, Result};
use owo_colors::OwoColorize;

use leyline::Response;

use crate::args::RequestArgs;
use crate::output::audit;

pub fn render(method: &str, url: &str, resp: &Response, args: &RequestArgs) -> Result<()> {
    let mut out = anstream::stdout().lock();

    // `--audit-only` means: skip the whole HTTP transcript and just
    // print the differentiated (fingerprint/tls/wire) blocks. The
    // request line, status line, response headers, and body are all
    // suppressed, matching the `--help` text.
    if !args.audit_only {
        // Request line.
        writeln!(out, "{}", format!("{method} {url}").dimmed()).context("writing request line")?;

        // Status line.
        let version = resp.version().as_str();
        let status_fmt = format_status(resp.status());
        writeln!(out, "{version} {status_fmt}").context("writing status line")?;

        // Response headers.
        for (k, v) in resp.headers() {
            writeln!(out, "{}: {v}", k.cyan()).ok();
        }
        writeln!(out).ok();

        // Body (unless suppressed by `-o`, which has already written
        // the bytes to disk via `maybe_write_body_to_file`).
        if args.output.is_none() {
            write_body(&mut out, resp).context("writing body")?;
        }
    }

    // Differentiated blocks — rendered regardless of audit_only.
    audit::render_blocks(&mut out, resp, args).context("writing audit blocks")?;

    Ok(())
}

/// Returns the status code colored by class: 2xx green, 3xx cyan,
/// 4xx yellow, 5xx red, 1xx default. Always bold. Omits the trailing
/// space when the reason phrase is unknown so the line renders
/// cleanly in snapshot tests.
fn format_status(status: u16) -> String {
    let reason = reason_phrase(status);
    let line = if reason.is_empty() {
        status.to_string()
    } else {
        format!("{status} {reason}")
    };
    match status {
        200..=299 => line.green().bold().to_string(),
        300..=399 => line.cyan().bold().to_string(),
        400..=499 => line.yellow().bold().to_string(),
        500..=599 => line.red().bold().to_string(),
        _ => line.bold().to_string(),
    }
}

/// A tiny hand-maintained reason-phrase table. Good enough for the
/// common cases; unknown codes render with an empty reason.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Detect content type, attempt JSON colorizing if it parses, fall
/// back to raw-bytes-lossy otherwise.
fn write_body<W: Write>(out: &mut W, resp: &Response) -> std::io::Result<()> {
    let body = resp.bytes();
    if body.is_empty() {
        return Ok(());
    }
    let ct = resp.content_type().unwrap_or("");
    let looks_json = ct.contains("json") || ct.contains("+json") || looks_like_json(body);

    if looks_json {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
            return color_json(out, &pretty);
        }
    }

    // Fallback — UTF-8 lossy.
    let text = String::from_utf8_lossy(body);
    out.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        writeln!(out)?;
    }
    Ok(())
}

fn looks_like_json(body: &[u8]) -> bool {
    body.iter()
        .find(|b| !b.is_ascii_whitespace())
        .map(|b| matches!(*b, b'{' | b'['))
        .unwrap_or(false)
}

/// Minimal JSON colorizer — walks the pre-pretty-printed string
/// character-by-character and paints strings/numbers/keywords. Avoids
/// pulling in syntect.
fn color_json<W: Write>(out: &mut W, s: &str) -> std::io::Result<()> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut after_colon = false; // tracks whether the next string is a value or a key

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'"' => {
                // Scan to the matching close-quote, respecting escapes.
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() {
                    i += 1;
                }
                let token = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                if after_colon {
                    write!(out, "{}", token.green())?;
                } else {
                    write!(out, "{}", token.cyan().bold())?;
                }
                after_colon = false;
            }
            b':' => {
                write!(out, ":")?;
                after_colon = true;
                i += 1;
            }
            b',' => {
                write!(out, ",")?;
                after_colon = false;
                i += 1;
            }
            b't' | b'f' | b'n' if is_keyword_at(bytes, i) => {
                let (tok, len) = keyword_at(bytes, i);
                write!(out, "{}", tok.yellow())?;
                i += len;
                after_colon = false;
            }
            b'0'..=b'9' | b'-' => {
                let start = i;
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    i += 1;
                }
                let token = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                write!(out, "{}", token.magenta())?;
                after_colon = false;
            }
            _ => {
                out.write_all(&[c])?;
                i += 1;
            }
        }
    }
    if !s.ends_with('\n') {
        writeln!(out)?;
    }
    Ok(())
}

fn is_keyword_at(bytes: &[u8], i: usize) -> bool {
    bytes[i..].starts_with(b"true")
        || bytes[i..].starts_with(b"false")
        || bytes[i..].starts_with(b"null")
}

fn keyword_at(bytes: &[u8], i: usize) -> (&str, usize) {
    if bytes[i..].starts_with(b"true") {
        ("true", 4)
    } else if bytes[i..].starts_with(b"false") {
        ("false", 5)
    } else {
        ("null", 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_phrases_cover_the_common_cases() {
        assert_eq!(reason_phrase(200), "OK");
        assert_eq!(reason_phrase(404), "Not Found");
        assert_eq!(reason_phrase(429), "Too Many Requests");
        assert_eq!(reason_phrase(999), "");
    }

    #[test]
    fn looks_like_json_matches_json_bodies() {
        assert!(looks_like_json(b"{\"a\":1}"));
        assert!(looks_like_json(b"   [1,2,3]"));
        assert!(!looks_like_json(b"<html>"));
        assert!(!looks_like_json(b""));
    }
}
