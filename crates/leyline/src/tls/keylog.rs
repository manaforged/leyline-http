#[cfg(debug_assertions)]
use std::fs::OpenOptions;
#[cfg(debug_assertions)]
use std::io::Write;
#[cfg(debug_assertions)]
use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::SslContextBuilder;

#[cfg(not(debug_assertions))]
pub(crate) fn install_from_env(_builder: &mut SslContextBuilder) {}

#[cfg(debug_assertions)]
pub(crate) fn install_from_env(builder: &mut SslContextBuilder) {
    if !cfg!(debug_assertions) {
        return;
    }
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

#[cfg(debug_assertions)]
fn keylog_writer(path: &str) -> std::io::Result<impl Fn(&str) + Send + Sync + 'static> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let file = Arc::new(Mutex::new(file));
    Ok(move |line: &str| {
        let mut file = crate::util::lock(&file);
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    })
}

#[cfg(test)]
mod tests;
