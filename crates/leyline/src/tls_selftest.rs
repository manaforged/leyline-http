//! Boot-time TLS handshake canary.

#![allow(dead_code, reason = "operator canary; no crate caller yet")]

use std::time::Duration;

use crate::{Browser, Session};

/// Default canary URL.
pub const DEFAULT_CANARY_URL: &str = "https://www.google.com/generate_204";

/// Spawn a background task that opens one TLS handshake to the canary URL and logs the outcome.
pub fn spawn_canary(service: &'static str) {
    spawn_canary_to(service, DEFAULT_CANARY_URL);
}

/// Same as [`spawn_canary`] but lets the caller override the probe target — useful for integration tests and for on-host verification against a private CA.
pub fn spawn_canary_to(service: &'static str, url: &'static str) {
    tokio::spawn(async move {
        let session = match Session::builder()
            .browser(Browser::Chrome147)
            .http1()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    target: "leyline::tls_selftest",
                    service,
                    err = %e,
                    "tls_selftest: session build failed (trust store wiring broken?)"
                );
                return;
            }
        };

        match session.get(url).await {
            Ok(resp) => {
                tracing::info!(
                    target: "leyline::tls_selftest",
                    service,
                    url,
                    status = resp.status(),
                    "tls_selftest: handshake OK"
                );
            }
            Err(e) => {
                tracing::error!(
                    target: "leyline::tls_selftest",
                    service,
                    url,
                    err = %e,
                    "tls_selftest: handshake failed — outbound HTTPS is broken"
                );
            }
        }
    });
}
