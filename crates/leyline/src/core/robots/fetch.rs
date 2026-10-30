use http::StatusCode;

use super::{ROBOTS_PATH, Robots, RobotsSource};
use crate::core::config::Redirect;
use crate::core::error::Result;
use crate::core::into_url::IntoUrl;
use crate::core::retry::Retry;
use crate::core::session::Session;

const MAX_REDIRECTS: usize = 5;
const MAX_BYTES: usize = 512 * 1024;

impl Session {
    /// Fetches and parses the site's robots.txt for `agent`, with 3 retries and up to 5 redirects.
    pub async fn robots(&self, url: impl IntoUrl, agent: &str) -> Result<Robots> {
        self.robots_with(url, agent, Retry::new(3), None).await
    }

    pub(crate) async fn robots_with(
        &self,
        url: impl IntoUrl,
        agent: &str,
        retry: Retry,
        trusted: Option<url::Url>,
    ) -> Result<Robots> {
        let target = url.into_url_with_base(self.base_url())?.join(ROBOTS_PATH)?;
        let mut builder = self
            .get(target.as_str())
            .redirect(Redirect::limit(MAX_REDIRECTS))
            .retry(retry);
        builder.trusted_origin = trusted;
        let response = match builder.stream().send().await {
            Ok(response) => response,
            Err(err) if err.is_shut_down() => return Err(err),
            Err(_) => return Ok(Robots::empty(RobotsSource::Unreachable)),
        };
        let status = response.status();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(Robots::empty(RobotsSource::Unreachable));
        }
        if !status.is_success() {
            return Ok(Robots::empty(RobotsSource::Unavailable));
        }
        let body = async {
            response
                .read_until(MAX_BYTES, |_, _| false)
                .await
                .map(|(bytes, _)| bytes)
        };
        match self.deadline(None).total(body).await {
            Ok(bytes) => {
                let body = String::from_utf8_lossy(&bytes);
                let whole = match bytes.len() >= MAX_BYTES {
                    true => body.rfind('\n').map_or("", |end| &body[..end]),
                    false => &body,
                };
                Ok(Robots::parse(whole, agent))
            }
            Err(err) if err.is_shut_down() => Err(err),
            Err(_) => Ok(Robots::empty(RobotsSource::Unreachable)),
        }
    }
}
