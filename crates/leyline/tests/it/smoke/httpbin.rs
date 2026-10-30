use std::fmt;
use std::future::Future;
use std::time::Duration;

use http::StatusCode;
use leyline::{Browser, Error, Kind, Response, Session, Timeout};
use serde_json::Value;

use super::Result;

const HTTPBIN: &str = "https://httpbin.org";
pub(super) const HTTPBIN_ATTEMPTS: u32 = 3;
const HTTPBIN_BACKOFF: Duration = Duration::from_millis(200);
const HTTPBIN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug)]
pub(super) struct HttpbinUnavailable {
    reason: String,
}

impl fmt::Display for HttpbinUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for HttpbinUnavailable {}

#[derive(Debug)]
pub(super) enum HttpbinAttempt {
    Unavailable(String),
    Fatal(String),
}

impl HttpbinAttempt {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable(reason.into())
    }

    fn fatal(reason: impl Into<String>) -> Self {
        Self::Fatal(reason.into())
    }
}

impl From<Error> for HttpbinAttempt {
    fn from(err: Error) -> Self {
        if httpbin_transport_down(&err) {
            Self::unavailable(err.to_string())
        } else {
            Self::fatal(err.to_string())
        }
    }
}

fn httpbin_transport_down(err: &Error) -> bool {
    err.kind() == Kind::Timeout || matches!(err.kind(), Kind::Connect | Kind::Dns)
}

pub(super) fn httpbin_budget() -> Duration {
    let gaps = HTTPBIN_ATTEMPTS.saturating_sub(1);
    let pause = HTTPBIN_BACKOFF.saturating_mul(gaps * HTTPBIN_ATTEMPTS / 2);
    HTTPBIN_ATTEMPT_TIMEOUT
        .saturating_mul(HTTPBIN_ATTEMPTS)
        .saturating_add(pause)
        .saturating_add(Duration::from_secs(3))
}

fn httpbin_session() -> leyline::Result<Session> {
    session_with_timeout(HTTPBIN_ATTEMPT_TIMEOUT)
}

pub(super) fn session_with_timeout(total: Duration) -> leyline::Result<Session> {
    Session::builder()
        .browser(Browser::default())
        .timeout(Timeout::new().total(total).connect(total))
        .build()
}

pub(super) async fn with_httpbin<F, Fut>(op: F) -> Result<String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<String, HttpbinAttempt>>,
{
    attempt_httpbin(HTTPBIN_ATTEMPTS, HTTPBIN_BACKOFF, op).await
}

pub(super) async fn attempt_httpbin<F, Fut>(
    attempts: u32,
    backoff: Duration,
    mut op: F,
) -> Result<String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<String, HttpbinAttempt>>,
{
    let attempts = attempts.max(1);
    let mut last = String::from("no response");
    for n in 1..=attempts {
        match op().await {
            Ok(detail) => return Ok(detail),
            Err(HttpbinAttempt::Fatal(reason)) => return Err(reason.into()),
            Err(HttpbinAttempt::Unavailable(reason)) => {
                last = reason;
                if n < attempts {
                    println!("    retry {n}/{attempts} after {last}");
                    tokio::time::sleep(backoff.saturating_mul(n)).await;
                }
            }
        }
    }
    Err(Box::new(HttpbinUnavailable {
        reason: format!("httpbin unavailable after {attempts} attempts: {last}"),
    }))
}

pub(super) fn expect_httpbin_status(
    status: StatusCode,
    expected: StatusCode,
) -> std::result::Result<(), HttpbinAttempt> {
    if status == expected {
        Ok(())
    } else if status.is_server_error() {
        Err(HttpbinAttempt::unavailable(format!(
            "status={}",
            status.as_u16()
        )))
    } else {
        Err(HttpbinAttempt::fatal(format!("status={}", status.as_u16())))
    }
}

pub(super) fn httpbin_eq(
    condition: bool,
    message: impl Into<String>,
) -> std::result::Result<(), HttpbinAttempt> {
    if condition {
        Ok(())
    } else {
        Err(HttpbinAttempt::fatal(message))
    }
}

fn httpbin_str<'a>(value: &'a Value, name: &str) -> std::result::Result<&'a str, HttpbinAttempt> {
    value
        .as_str()
        .ok_or_else(|| HttpbinAttempt::fatal(format!("missing {name}")))
}

pub(super) async fn read_httpbin_text(
    response: Response,
    expected: StatusCode,
) -> std::result::Result<String, HttpbinAttempt> {
    expect_httpbin_status(response.status(), expected)?;
    Ok(response.text().await?)
}

pub(super) async fn read_httpbin_json(
    response: Response,
) -> std::result::Result<Value, HttpbinAttempt> {
    expect_httpbin_status(response.status(), StatusCode::OK)?;
    match response.json().await {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == Kind::Json => {
            Err(HttpbinAttempt::unavailable(format!("non-JSON body: {err}")))
        }
        Err(err) => Err(err.into()),
    }
}

pub(super) async fn httpbin_cdn_get() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get(format!("{HTTPBIN}/get")).send().await?;
        let body = read_httpbin_text(response, StatusCode::OK).await?;
        httpbin_eq(body.contains("headers"), "httpbin body missing headers")?;
        Ok(format!("status=200 body={}B", body.len()))
    })
    .await
}

pub(super) async fn httpbin_post_json() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let payload = serde_json::json!({"test": "leyline", "v": 2});
        let response = session
            .post(format!("{HTTPBIN}/post"))
            .json(&payload)
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let echoed = httpbin_str(&value["json"]["test"], "json.test")?;
        httpbin_eq(echoed == "leyline", format!("echoed={echoed}"))?;
        Ok(format!("echoed={echoed}"))
    })
    .await
}

pub(super) async fn httpbin_post_form() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .post(format!("{HTTPBIN}/post"))
            .form([("user", "alice"), ("pass", "s3cret!")])
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let user = httpbin_str(&value["form"]["user"], "form.user")?;
        let pass = httpbin_str(&value["form"]["pass"], "form.pass")?;
        httpbin_eq(
            user == "alice" && pass == "s3cret!",
            format!("user={user} pass={pass}"),
        )?;
        Ok(format!("user={user} pass={pass}"))
    })
    .await
}

pub(super) async fn httpbin_query() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .request(http::Method::GET, format!("{HTTPBIN}/get"))
            .query([("foo", "bar"), ("n", "42")])
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let arg_foo = httpbin_str(&value["args"]["foo"], "args.foo")?;
        httpbin_eq(arg_foo == "bar", format!("foo={arg_foo}"))?;
        Ok(format!("foo={arg_foo}"))
    })
    .await
}

pub(super) async fn httpbin_bearer() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .request(http::Method::GET, format!("{HTTPBIN}/get"))
            .bearer_auth("test-token-123")
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let auth = httpbin_str(&value["headers"]["Authorization"], "headers.Authorization")?;
        httpbin_eq(auth.contains("test-token-123"), format!("auth={auth}"))?;
        Ok(format!("auth={auth}"))
    })
    .await
}

pub(super) async fn httpbin_status_404() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get(format!("{HTTPBIN}/status/404")).send().await?;
        expect_httpbin_status(response.status(), StatusCode::NOT_FOUND)?;
        httpbin_eq(response.error_for_status().is_err(), "404 should be error")?;
        Ok("404 -> Err".to_string())
    })
    .await
}

pub(super) async fn httpbin_redirect() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get(format!("{HTTPBIN}/redirect/2")).send().await?;
        expect_httpbin_status(response.status(), StatusCode::OK)?;
        let hops = response.redirect_chain().len();
        httpbin_eq(hops > 0, "redirect chain empty")?;
        Ok(format!("{hops} redirects"))
    })
    .await
}

pub(super) async fn httpbin_large() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get(format!("{HTTPBIN}/bytes/50000")).send().await?;
        expect_httpbin_status(response.status(), StatusCode::OK)?;
        let n = response.bytes().await?.len();
        httpbin_eq(n == 50_000, format!("got {n} bytes"))?;
        Ok(format!("{n}B"))
    })
    .await
}
