//! `RequestArgs` → `leyline::Session` glue.
//!
//! The only reason this module exists is so `cmd/request.rs` and
//! `cmd/inspect.rs` can reuse a single translation path without
//! duplicating flag handling.

use std::io::IsTerminal;

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use owo_colors::OwoColorize;
use tokio::io::AsyncReadExt;

use leyline::{RequestBuilder, Response, Session};

use crate::args::{OutputFormat, RequestArgs};

/// Build a `leyline::Session` from CLI flags.
pub fn build_session(args: &RequestArgs) -> Result<Session> {
    let browser = args.resolve_browser().map_err(|e| anyhow!(e))?;
    let platform = args.resolve_platform().map_err(|e| anyhow!(e))?;

    let mut builder = Session::builder().browser(browser).platform(platform);

    if let Some(proxy) = &args.proxy {
        builder = builder.proxy(proxy);
    }
    if let Some(timeout) = args.timeout {
        // One `--timeout` flag in the CLI, one setter on the library.
        // We pin it to the session builder so every request the CLI
        // issues (including any internal retries inside leyline) sees
        // the same ceiling. A later `RequestBuilder::timeout` call
        // would override it, but the CLI never makes one.
        builder = builder.timeout(timeout);
    }
    builder = builder.max_redirects(args.redirect);

    if args.http3 {
        builder = builder.http3();
    } else if args.http2 {
        builder = builder.http2();
    } else if args.http1 {
        builder = builder.http1();
    }

    if args.insecure {
        // Write through anstream so NO_COLOR / pipe redirection is
        // honored. The warning is intentionally loud — anyone reading
        // CI output should see it immediately.
        let mut stderr = anstream::stderr().lock();
        let _ = std::io::Write::write_all(
            &mut stderr,
            format!(
                "{} TLS peer verification disabled (-k/--insecure). \
                 Any MITM can serve arbitrary content without detection.\n",
                "leyline: warning —".yellow().bold()
            )
            .as_bytes(),
        );
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build().context("session build failed")
}

/// Apply per-request flags (headers, body, auth, query) to a
/// `RequestBuilder`. Consumes and returns it so the caller can chain.
///
/// Async because `-d -` / `-d @file` read from stdin / disk via
/// `tokio::io`, avoiding a block on the runtime.
pub async fn apply_request_args<'a>(
    mut rb: RequestBuilder<'a>,
    args: &RequestArgs,
) -> Result<RequestBuilder<'a>> {
    // Query parameters.
    if !args.query.is_empty() {
        let pairs: Vec<(&str, &str)> = args
            .query
            .iter()
            .map(|kv| split_kv(kv, '='))
            .collect::<Result<_>>()?;
        rb = rb.query(&pairs);
    }

    // Auth — basic first, bearer wins if both are given.
    if let Some(user) = &args.user {
        let (u, p) = split_kv(user, ':').unwrap_or((user.as_str(), ""));
        rb = rb.basic_auth(u, p);
    }
    if let Some(token) = &args.bearer {
        rb = rb.bearer_auth(token);
    }

    // Extra headers. Parse `Name: value`, preserve user-supplied order.
    for raw in &args.header {
        let (name, value) = raw
            .split_once(':')
            .ok_or_else(|| anyhow!("header must be `Name: value`, got `{raw}`"))?;
        rb = rb.append_header(name.trim(), value.trim_start());
    }

    // Body is mutually exclusive: --json > --form > --data.
    if let Some(json_raw) = &args.json {
        let value: serde_json::Value =
            serde_json::from_str(json_raw).context("--json must be valid JSON")?;
        rb = rb.json(&value);
    } else if !args.form.is_empty() {
        let pairs: Vec<(&str, &str)> = args
            .form
            .iter()
            .map(|kv| split_kv(kv, '='))
            .collect::<Result<_>>()?;
        rb = rb.form(&pairs);
    } else if let Some(data) = &args.data {
        let body = read_data_arg(data).await?;
        rb = rb.body(body);
    }

    Ok(rb)
}

/// Cap for `-d @file` / `-d -` bodies so we don't accidentally load
/// a 50 GB log into memory.
pub const MAX_DATA_ARG_BYTES: u64 = 128 * 1024 * 1024;

/// Read a `-d/--data` argument: literal text, `@path`, or `-` for stdin.
pub async fn read_data_arg(spec: &str) -> Result<Vec<u8>> {
    if spec == "-" {
        let mut buf = Vec::new();
        let mut stdin = tokio::io::stdin();
        stdin
            .read_to_end(&mut buf)
            .await
            .context("reading body from stdin")?;
        if (buf.len() as u64) > MAX_DATA_ARG_BYTES {
            return Err(anyhow!(
                "stdin body exceeds {} MiB cap (pass the body via `-d @file` and chunk it instead)",
                MAX_DATA_ARG_BYTES / (1024 * 1024)
            ));
        }
        Ok(buf)
    } else if let Some(path) = spec.strip_prefix('@') {
        let meta = tokio::fs::metadata(path)
            .await
            .with_context(|| format!("opening {path}"))?;
        if meta.len() > MAX_DATA_ARG_BYTES {
            return Err(anyhow!(
                "{path} is {} bytes, exceeds the {} MiB cap for `-d @file`",
                meta.len(),
                MAX_DATA_ARG_BYTES / (1024 * 1024)
            ));
        }
        tokio::fs::read(path)
            .await
            .with_context(|| format!("reading body from {path}"))
    } else {
        Ok(spec.as_bytes().to_vec())
    }
}

/// Pick the effective output format.
///
/// Explicit `--format` wins. Otherwise `pretty` when stdout is a TTY
/// or when the user asked for any differentiated block (`-v`,
/// `--show-fingerprint`, `--show-cert`, `--show-wire`, `--audit-only`)
/// — dropping to `raw` in those cases would silently eat exactly the
/// thing the user asked for. Default to `raw` when stdout is piped
/// and no differentiated flag is set, so `leyline get url | jq` works
/// out of the box.
pub fn effective_format(args: &RequestArgs) -> OutputFormat {
    if let Some(fmt) = args.format {
        return fmt;
    }
    let wants_audit = args.verbose
        || args.show_fingerprint
        || args.show_cert.is_some()
        || args.show_wire
        || args.audit_only;
    if wants_audit || std::io::stdout().is_terminal() {
        OutputFormat::Pretty
    } else {
        OutputFormat::Raw
    }
}

/// Split `"k=v"` or `"user:pass"` into `(k, v)`. Returns `Err` if the
/// delimiter is missing.
fn split_kv(raw: &str, delim: char) -> Result<(&str, &str)> {
    raw.split_once(delim)
        .ok_or_else(|| anyhow!("expected `key{delim}value`, got `{raw}`"))
}

/// Base64-encode bytes. Used by the JSON output renderer.
pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Write the response body to `-o <file>`, or leave it to the caller
/// otherwise. Returns `true` when the body has already been consumed
/// and the renderer should skip printing it.
///
/// `--output -` means stdout, matching the usual Unix convention.
pub fn maybe_write_body_to_file(resp: &Response, args: &RequestArgs) -> Result<bool> {
    let Some(path) = args.output.as_ref() else {
        return Ok(false);
    };
    if path.as_os_str() == "-" {
        use std::io::Write;
        std::io::stdout()
            .lock()
            .write_all(resp.bytes())
            .context("writing body to stdout")?;
        return Ok(true);
    }
    std::fs::write(path, resp.bytes())
        .with_context(|| format!("writing body to {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::RequestArgs;

    #[tokio::test]
    async fn read_data_literal_text() {
        assert_eq!(read_data_arg("hello").await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn read_data_from_file() {
        let dir = tempdir();
        let path = dir.join("body.txt");
        std::fs::write(&path, b"file body").unwrap();
        let spec = format!("@{}", path.display());
        assert_eq!(read_data_arg(&spec).await.unwrap(), b"file body");
    }

    #[tokio::test]
    async fn read_data_file_enforces_size_cap() {
        let dir = tempdir();
        let path = dir.join("huge.bin");
        // Create a sparse file larger than the cap without actually
        // writing a GB of zeros: just set_len on an empty file.
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_DATA_ARG_BYTES + 1).unwrap();
        let spec = format!("@{}", path.display());
        let err = read_data_arg(&spec).await.unwrap_err();
        assert!(
            err.to_string().contains("exceeds"),
            "expected size-cap error, got: {err}"
        );
    }

    #[test]
    fn split_kv_parses_forms() {
        assert_eq!(split_kv("k=v", '=').unwrap(), ("k", "v"));
        assert_eq!(split_kv("k=v=more", '=').unwrap(), ("k", "v=more"));
        assert!(split_kv("no-delim", '=').is_err());
    }

    #[test]
    fn effective_format_respects_explicit_override() {
        let args = RequestArgs {
            format: Some(OutputFormat::Json),
            ..Default::default()
        };
        assert_eq!(effective_format(&args), OutputFormat::Json);
    }

    #[test]
    fn effective_format_forces_pretty_when_audit_opt_in_and_piped() {
        // We can't simulate a non-TTY stdout here, but we can verify
        // that `wants_audit` wins regardless of the terminal state by
        // checking every flag pathway. The condition is an `||`, so
        // any of these by itself should force Pretty.
        let cases = [
            RequestArgs {
                verbose: true,
                ..Default::default()
            },
            RequestArgs {
                show_fingerprint: true,
                ..Default::default()
            },
            RequestArgs {
                show_wire: true,
                ..Default::default()
            },
            RequestArgs {
                audit_only: true,
                ..Default::default()
            },
            RequestArgs {
                show_cert: Some(crate::args::CertFormat::Summary),
                ..Default::default()
            },
        ];
        for args in cases {
            assert_eq!(
                effective_format(&args),
                OutputFormat::Pretty,
                "audit opt-in should force Pretty for {args:?}"
            );
        }
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "leyline-cli-test-{}",
            std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
