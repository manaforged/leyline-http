//! Boot-time TLS handshake canary.
//!
//! A trust-store regression (CERTIFICATE_VERIFY_FAILED) can look
//! healthy at the process level — a `/health` endpoint returns 200 and
//! the listener keeps accepting — because nothing in the startup path
//! actually exercises an outbound TLS handshake. A service is
//! technically listening, but every request 500s the moment it tries to
//! reach an HTTPS origin. A circuit breaker can't see that until real
//! traffic fails.
//!
//! `spawn_canary` closes that gap: immediately after the listener binds,
//! the service spawns a one-shot task that performs a real TLS GET to
//! a known-good HTTPS endpoint. Failure logs at `error!` so grep-alerts
//! on `tls_selftest: handshake failed` catch trust-store regressions
//! before they page anyone.
//!
//! Deliberately fire-and-forget:
//! - Non-fatal: a canary miss shouldn't prevent the service from
//!   starting up, because the canary target may itself be down while
//!   the workload's real destinations stay reachable.
//! - Single attempt: retries would obscure whether the underlying
//!   trust store is broken or the network is flaky.

use std::time::Duration;

use crate::{Browser, Session};

/// Default canary URL. `generate_204` has been stable since 2012, is
/// served by Google's global anycast frontend (so it's reachable from
/// every host including behind proxies), and returns an empty
/// 204 body so the log line records only the status.
pub const DEFAULT_CANARY_URL: &str = "https://www.google.com/generate_204";

/// Spawn a background task that opens one TLS handshake to the canary
/// URL and logs the outcome. Safe to call from any tokio context.
///
/// `service` is the caller's service name (e.g. `"api-gateway"`) — it
/// shows up in the log span so multi-service hosts can distinguish
/// whose canary ticked.
pub fn spawn_canary(service: &'static str) {
    spawn_canary_to(service, DEFAULT_CANARY_URL);
}

/// Same as [`spawn_canary`] but lets the caller override the probe
/// target — useful for integration tests and for on-host verification
/// against a private CA.
pub fn spawn_canary_to(service: &'static str, url: &'static str) {
    tokio::spawn(async move {
        // Force HTTP/1.1 — we only care whether the TLS handshake
        // completes against a trusted issuer. HTTP/2 on Google's
        // frontends sends a 12 KiB HPACK dynamic-table-size update
        // which our decoder rejects (max 4 KiB); that's a separate
        // leyline-h2 issue that would give us a false-negative canary
        // even on a healthy trust store.
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

        match session.get(url).send().await {
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
