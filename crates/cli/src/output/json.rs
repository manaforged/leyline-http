//! Structured JSON output — stable schema for scripting.
//!
//! The schema is documented in `leyline --help` and should not change
//! between minor releases without a migration note.

use std::io::Write;

use anyhow::{Context, Result};
use leyline::Response;
use serde_json::{json, Value};

use crate::args::RequestArgs;
use crate::session::b64;

pub fn render(method: &str, url: &str, resp: &Response, args: &RequestArgs) -> Result<()> {
    let doc = build_document(method, url, resp, args);
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &doc).context("writing JSON output")?;
    stdout.write_all(b"\n").ok();
    Ok(())
}

fn build_document(method: &str, url: &str, resp: &Response, args: &RequestArgs) -> Value {
    let request = json!({
        "method": method,
        "url": url,
        "headers_final": header_pairs(resp.request_headers()),
    });

    let mut response = json!({
        "status": resp.status(),
        "version": resp.version().as_str(),
        "url": resp.url(),
        "redirect_chain": resp.redirect_chain(),
        "headers": header_pairs(resp.headers()),
    });
    if !args.audit_only {
        response["body_base64"] = Value::String(b64(resp.bytes()));
    }

    let audit = resp.audit().map(|a| {
        json!({
            "ja4": a.ja4,
            "ja3": a.ja3,
            "h2_fingerprint": a.h2_fingerprint,
            "ja4t": a.ja4t,
            "ja4h": a.ja4h,
        })
    });

    let tls = json!({
        "alpn": resp.tls_alpn(),
        "version": resp.tls_version(),
        "cipher": resp.tls_cipher(),
        "peer_cert_der_base64": resp.tls_peer_certificate().map(b64),
    });

    json!({
        "request": request,
        "response": response,
        "audit": audit,
        "tls": tls,
    })
}

fn header_pairs(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), Value::String(v.clone())]))
            .collect(),
    )
}
