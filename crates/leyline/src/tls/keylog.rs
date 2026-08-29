//! Opt-in TLS secret logging for wire debugging (curl/browser compatible).
//!
//! When `SSLKEYLOGFILE` is set, per-connection TLS secrets are appended to
//! the named file in the NSS key-log format Wireshark consumes — the same
//! mechanism curl, Firefox, and Chrome honour. This is how every capture in
//! `tests/` documentation was debuggable without a patched client.
//!
//! Secrets written here decrypt the traffic they belong to. The mechanism is
//! env opt-in only, the activation is logged at `warn` (matching how
//! `SSL_CERT_FILE` trust overrides are surfaced), and the file is opened in
//! append mode so parallel sessions and processes interleave safely.
//! Unset (the default) costs one `env::var` lookup per context build.
//!
//! Threading note: the callback writes synchronously inside the handshake,
//! on whatever thread drives it (including tokio workers on the async
//! path). Writes are a few lines, once per secret, and flushed — the same
//! trade curl and the browsers make. Heavy capture pipelines should point
//! `SSLKEYLOGFILE` at a tmpfs file.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::SslContextBuilder;

/// Install the NSS key-log writer on `builder` when `SSLKEYLOGFILE` is set.
pub(crate) fn install_from_env(builder: &mut SslContextBuilder) {
    let Ok(path) = std::env::var("SSLKEYLOGFILE") else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    match keylog_writer(&path) {
        Ok(writer) => {
            builder.set_keylog_callback(move |_ssl, line| writer(line));
            tracing::warn!(
                target: "leyline::tls::trust",
                keylog_file = %path,
                "SSLKEYLOGFILE honoured — TLS secrets are being written to disk; \
                 unset the variable to disable"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "leyline::tls::trust",
                keylog_file = %path,
                %error,
                "SSLKEYLOGFILE could not be opened; key logging disabled"
            );
        }
    }
}

/// Open `path` for appending and return the line-writer closure installed as
/// the BoringSSL keylog callback. Each line is flushed immediately so a
/// capture being decrypted live sees secrets as handshakes complete.
fn keylog_writer(path: &str) -> std::io::Result<impl Fn(&str) + Send + Sync + 'static> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let file = Arc::new(Mutex::new(file));
    Ok(move |line: &str| {
        let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    })
}

#[cfg(test)]
mod tests;
