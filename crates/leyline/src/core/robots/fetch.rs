use super::{Robots, RobotsSource};
use crate::core::config::RedirectPolicy;
use crate::core::error::Result;
use crate::core::into_url::IntoUrl;
use crate::core::session::Session;

const ROBOTS_FILE: &str = "/robots.txt";
const MAX_REDIRECTS: usize = 5;
const MAX_BYTES: usize = 512 * 1024;

impl Session {
    pub async fn robots(&self, url: impl IntoUrl, agent: &str) -> Result<Robots> {
        let target = url.into_url_with_base(self.base_url())?.join(ROBOTS_FILE)?;
        let response = match self
            .get(target.as_str())
            .redirect(RedirectPolicy::limited(MAX_REDIRECTS))
            .stream()
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) if err.is_shut_down() => return Err(err),
            Err(_) => return Ok(Robots::empty(RobotsSource::Unreachable)),
        };
        let status = response.status();
        if status.is_server_error() {
            return Ok(Robots::empty(RobotsSource::Unreachable));
        }
        if !status.is_success() {
            return Ok(Robots::empty(RobotsSource::Unavailable));
        }
        match response.read_until(MAX_BYTES, |_, _| false).await {
            Ok(bytes) => Ok(Robots::parse(&String::from_utf8_lossy(&bytes), agent)),
            Err(err) if err.is_shut_down() => Err(err),
            Err(_) => Ok(Robots::empty(RobotsSource::Unreachable)),
        }
    }
}
