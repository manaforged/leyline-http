//! Opt-in TLS secret logging for wire debugging (curl/browser compatible).

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

/// Open `path` for appending and return the line-writer closure installed as the BoringSSL keylog callback.
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
