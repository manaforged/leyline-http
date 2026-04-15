//! Fingerprint / TLS / wire-header block renderer.
//!
//! Used by both the `pretty` renderer and the `inspect` subcommand.

use std::io::Write;

use owo_colors::OwoColorize;

use leyline::Response;

use crate::args::{CertFormat, RequestArgs};

/// Write the differentiated blocks (fingerprint, tls, wire, cert) to
/// `out`, respecting the `--show-*` flags on `args`.
///
/// `--audit-only` turns on the fingerprint and TLS blocks implicitly
/// (since the whole point is to show fingerprint context). It does
/// **not** turn off `--show-wire` — the flags are orthogonal. A user
/// who passes `-v --audit-only` still gets the wire headers.
pub fn render_blocks<W: Write>(
    out: &mut W,
    resp: &Response,
    args: &RequestArgs,
) -> std::io::Result<()> {
    if args.show_fingerprint || args.audit_only {
        fingerprint_block(out, resp)?;
    }

    if args.show_cert.is_some() || args.audit_only {
        tls_block(out, resp, args.show_cert.unwrap_or(CertFormat::Summary))?;
    }

    if args.show_wire {
        wire_block(out, resp)?;
    }

    Ok(())
}

fn fingerprint_block<W: Write>(out: &mut W, resp: &Response) -> std::io::Result<()> {
    let Some(audit) = resp.audit() else {
        return Ok(());
    };
    divider(out, "fingerprint")?;
    row(out, "JA4 ", &audit.ja4.magenta().bold().to_string())?;
    row(out, "JA3 ", &audit.ja3.dimmed().to_string())?;
    row(out, "H2  ", &audit.h2_fingerprint.cyan().to_string())?;
    row(out, "JA4T", &audit.ja4t.cyan().to_string())?;
    row(out, "JA4H", &audit.ja4h.cyan().to_string())?;
    writeln!(out)?;
    Ok(())
}

fn tls_block<W: Write>(out: &mut W, resp: &Response, fmt: CertFormat) -> std::io::Result<()> {
    divider(out, "tls")?;
    if let Some(v) = resp.tls_version() {
        row(out, "version", &v.green().to_string())?;
    }
    if let Some(c) = resp.tls_cipher() {
        row(out, "cipher ", &c.green().to_string())?;
    }
    if let Some(a) = resp.tls_alpn() {
        row(out, "alpn   ", &a.green().to_string())?;
    }
    if let Some(der) = resp.tls_peer_certificate() {
        match fmt {
            CertFormat::Summary => {
                row(
                    out,
                    "peer   ",
                    &format!("{} DER bytes", der.len()).green().to_string(),
                )?;
            }
            CertFormat::Hex => {
                writeln!(out, "{}", "peer cert (hex):".dimmed())?;
                for chunk in der.chunks(32) {
                    let hex: String = chunk.iter().map(|b| format!("{b:02x}")).collect();
                    writeln!(out, "  {hex}")?;
                }
            }
            CertFormat::Pem => {
                let b64 = crate::session::b64(der);
                writeln!(out, "-----BEGIN CERTIFICATE-----")?;
                for chunk in b64.as_bytes().chunks(64) {
                    writeln!(out, "{}", std::str::from_utf8(chunk).unwrap_or(""))?;
                }
                writeln!(out, "-----END CERTIFICATE-----")?;
            }
        }
    }
    writeln!(out)?;
    Ok(())
}

fn wire_block<W: Write>(out: &mut W, resp: &Response) -> std::io::Result<()> {
    divider(out, "wire")?;
    for (k, v) in resp.request_headers() {
        writeln!(out, "{}: {}", k.cyan(), v)?;
    }
    writeln!(out)?;
    Ok(())
}

fn divider<W: Write>(out: &mut W, label: &str) -> std::io::Result<()> {
    let line = format!("── {label} ────────────────────────────────────");
    writeln!(out, "{}", line.dimmed())
}

fn row<W: Write>(out: &mut W, key: &str, value: &str) -> std::io::Result<()> {
    writeln!(out, "{}   {value}", key.dimmed())
}
