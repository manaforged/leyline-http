use std::future::Future;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;

/// The shortest pause between two requests to a third-party endpoint.
pub const GAP: Duration = Duration::from_secs(2);

/// How many times one live check may ask an endpoint.
pub const TRIES: u32 = 2;

const PUBLIC: &str = "https://tls.peet.ws/api/all";

static LAST: Mutex<Option<Instant>> = Mutex::const_new(None);

/// The fingerprint echo endpoint: `LEYLINE_ECHO_URL` when set, otherwise the public one.
pub fn url() -> String {
    std::env::var("LEYLINE_ECHO_URL").unwrap_or_else(|_| PUBLIC.to_owned())
}

/// Runs `request` alone, and at least `GAP` after the previous one when the endpoint is public.
pub async fn paced<T>(request: impl Future<Output = T>) -> T {
    let mut last = LAST.lock().await;
    if let Some(at) = *last
        && url() == PUBLIC
    {
        tokio::time::sleep_until((at + GAP).into()).await;
    }
    let out = request.await;
    *last = Some(Instant::now());
    out
}

/// Fetches the echo endpoint's JSON report of what `session` sent, trying at most `TRIES` times.
pub async fn get(session: &leyline::Session) -> Value {
    let mut last_err = String::new();
    for _ in 0..TRIES {
        match paced(report(session)).await {
            Ok(value) => return value,
            Err(error) => last_err = error,
        }
    }
    panic!("{} failed after {TRIES} tries: {last_err}", url());
}

async fn report(session: &leyline::Session) -> Result<Value, String> {
    let resp = session
        .get(url())
        .send()
        .await
        .map_err(|e| format!("request error: {e}"))?;
    if resp.status() != 200 {
        return Err(format!("status {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| format!("body: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("non-JSON: {e}"))
}
