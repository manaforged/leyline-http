use std::time::Duration;

use leyline::http::StatusCode;
use leyline::{Error, ErrorCategory, RetryPolicy, Session, WaitFormat};
use serde::{Deserialize, Serialize};

const BASE_URL: &str = "https://api.example/v1/";
const TOKEN_VAR: &str = "API_TOKEN";
const RATE_REMAINING: &str = "x-ratelimit-remaining";
const RATE_RESET: &str = "x-ratelimit-reset";
const MAX_RATE_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize)]
struct Repo {
    name: String,
}

#[derive(Serialize)]
struct NewIssue<'a> {
    title: &'a str,
}

struct Client {
    session: Session,
}

impl Client {
    fn new(token: &str) -> leyline::Result<Self> {
        let retry = RetryPolicy::transient()
            .max_retries(3)
            .retry_if(|resp| {
                resp.status() == StatusCode::FORBIDDEN && resp.header(RATE_REMAINING) == Some("0")
            })
            .wait_header(RATE_RESET, WaitFormat::UnixSeconds)
            .max_retry_after(MAX_RATE_WAIT);
        let session = Session::builder()
            .base_url(BASE_URL)
            .bearer_auth(token)
            .retry(retry)
            .build()?;
        Ok(Self { session })
    }

    async fn repos(&self) -> leyline::Result<Vec<Repo>> {
        let mut all = Vec::new();
        let mut pages = self.session.get("repos").error_for_status().pages();
        while let Some(page) = pages.next().await {
            all.extend(page?.json::<Vec<Repo>>().await?);
        }
        Ok(all)
    }

    async fn open_issue(&self, repo: &str, title: &str) -> leyline::Result<()> {
        self.session
            .post(format!("repos/{repo}/issues"))
            .json(&NewIssue { title })
            .error_for_status()
            .send()
            .await
            .map(drop)
    }

    fn explain(e: &Error) -> String {
        match e.category() {
            ErrorCategory::Status => match e.retry_after() {
                Some(wait) if e.retries_exhausted() => {
                    format!("rate limited for {wait:?} after {} attempts", e.attempts())
                }
                Some(wait) => format!("busy, retry in {wait:?}"),
                None => {
                    let body = e.body_text().unwrap_or_default();
                    format!("api error {:?}: {body}", e.status())
                }
            },
            ErrorCategory::Timeout => "the api timed out".to_string(),
            ErrorCategory::Dns | ErrorCategory::Connect | ErrorCategory::Tls => {
                format!("network error: {e}")
            }
            other => format!("{other:?}: {e}"),
        }
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let token = std::env::var(TOKEN_VAR).unwrap_or_else(|_| "REPLACE_WITH_YOUR_TOKEN".into());
    let client = Client::new(&token)?;
    match client.repos().await {
        Ok(repos) => repos.iter().for_each(|r| println!("{}", r.name)),
        Err(e) => eprintln!("{}", Client::explain(&e)),
    }
    if let Err(e) = client.open_issue("demo", "hello").await {
        eprintln!("{}", Client::explain(&e));
    }
    Ok(())
}
