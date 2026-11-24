use std::future::Future;
#[cfg(feature = "html")]
use std::time::Duration;

use http::StatusCode;
use url::Url;

use super::{ROBOTS_PATH, Robots, RobotsSource};
use crate::core::Redirect;
use crate::core::into_url::IntoUrl;
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::retry::Retry;
#[cfg(feature = "html")]
use crate::core::retry::RetryPlan;
use crate::core::session::Session;
use crate::error::{Error, Result};

const DEFAULT_MAX_BYTES: usize = 512 * 1024;

fn get(
    session: &Session,
    url: &Url,
    redirect: Redirect,
    retry: Retry,
    trusted: Option<Url>,
) -> RequestBuilder {
    session
        .get(url.as_str())
        .redirect(redirect)
        .retry(retry)
        .trusting(trusted)
}

pub(crate) async fn read_capped(
    session: &Session,
    response: Response,
    cap: usize,
) -> Result<Vec<u8>> {
    let body = async {
        response
            .read_until(cap, |_, _| false)
            .await
            .map(|(bytes, _)| bytes)
    };
    session.deadline(None).total(body).await
}

#[cfg(feature = "html")]
pub(crate) enum Reply {
    Later(Duration),
    Sent(Result<Response>),
}

#[cfg(feature = "html")]
impl Reply {
    pub(crate) async fn send(
        session: &Session,
        url: &Url,
        retry: &Retry,
        attempt: u32,
        trusted: Option<Url>,
    ) -> Reply {
        let request = get(session, url, Redirect::none(), retry.once(), trusted);
        let result = request.stream().send().await;
        match retry.plan(&result, attempt) {
            RetryPlan::Backoff(wait) => Reply::Later(wait),
            _ => Reply::Sent(result),
        }
    }
}

pub(crate) fn fallback(err: Error) -> Result<Robots> {
    match err.is_shut_down() {
        true => Err(err),
        false => Ok(Robots::empty(RobotsSource::Unreachable)),
    }
}

fn empty_source(status: StatusCode) -> Option<RobotsSource> {
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Some(RobotsSource::Unreachable);
    }
    (!status.is_success()).then_some(RobotsSource::Unavailable)
}

fn parse_capped(bytes: &[u8], cap: usize, agent: &str) -> Robots {
    let body = String::from_utf8_lossy(bytes);
    let whole = match bytes.len() >= cap {
        true => body.rfind(['\n', '\r']).map_or("", |end| &body[..end]),
        false => &body,
    };
    Robots::parse(whole, agent)
}

impl Session {
    /// Fetches and parses the site's robots.txt for `agent`, using the session's policies or 3 retries and up to 5 redirects.
    pub fn robots(
        &self,
        url: impl IntoUrl,
        agent: &str,
    ) -> impl Future<Output = Result<Robots>> + Send {
        let parsed = url.into_url_with_base(self.base_url());
        let retry = self.default_retry().for_robots();
        async move {
            let url = parsed?;
            self.robots_with(url, agent, retry, None).await
        }
    }

    pub(crate) async fn robots_with(
        &self,
        url: impl IntoUrl,
        agent: &str,
        retry: Retry,
        trusted: Option<Url>,
    ) -> Result<Robots> {
        let target = url.into_url_with_base(self.base_url())?.join(ROBOTS_PATH)?;
        let cap = self
            .body_rules()
            .configured_max_body()
            .unwrap_or(DEFAULT_MAX_BYTES);
        let builder = get(
            self,
            &target,
            self.redirect_policy().for_robots(),
            retry,
            trusted,
        );
        let response = match builder.stream().send().await {
            Ok(response) => response,
            Err(err) => return fallback(err),
        };
        if let Some(source) = empty_source(response.status()) {
            return Ok(Robots::empty(source));
        }
        match read_capped(self, response, cap).await {
            Ok(bytes) => Ok(parse_capped(&bytes, cap, agent)),
            Err(err) => fallback(err),
        }
    }
}
