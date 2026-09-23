use super::RequestBuilder;
use crate::core::Result;
use crate::core::body::Body;
use crate::core::error::{Error, Kind};
use crate::core::response::Response;
use crate::core::session::execute::Attempt;
use crate::util::is_idempotent;

impl RequestBuilder {
    pub async fn send(mut self) -> Result<Response> {
        self.prepare()?;
        let retry_policy = self.retry_policy.clone();
        let digest_auth = self.digest_auth.take();
        let allow_non_idempotent_retry = self.allow_non_idempotent_retry;
        let timeout = self.timeout;
        let session = self.session.clone();
        let mut attempt = self.into_attempt();

        if retry_policy.is_none() && digest_auth.is_none() {
            return session.run(attempt).await;
        }

        let retryable_method = allow_non_idempotent_retry || is_idempotent(attempt.method.as_str());
        let body_retryable = !attempt.body.is_stream();
        let replay: Option<bytes::Bytes> = match &attempt.body {
            Body::Bytes(b) => Some(b.clone()),
            _ => None,
        };
        let base_headers = attempt.headers.clone();

        let session_timeout = timeout.unwrap_or_else(|| session.default_timeout());
        let deadline = tokio::time::Instant::now() + session_timeout;

        let mut n: u32 = 0;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(Kind::Timeout));
            }
            let is_stream_body = attempt.body.is_stream();
            let hop_body = std::mem::take(&mut attempt.body);
            let this = attempt.again(hop_body, base_headers.clone(), Some(remaining));
            let result = session.run(this).await;

            if let (Some(auth), Ok(resp)) = (&digest_auth, result.as_ref())
                && resp.status() == 401
                && n == 0
                && let Some(header) = resp.header("www-authenticate")
                && let Ok(challenge) = crate::core::digest::parse_challenge(header)
            {
                let parsed = url::Url::parse(resp.url())?;
                let uri_path = match parsed.query() {
                    Some(q) => format!("{}?{}", parsed.path(), q),
                    None => parsed.path().to_string(),
                };
                if is_stream_body {
                    return Err(Error::new(Kind::Request).with_message(
                        "digest auth: cannot replay streaming request body. \
                         Buffer the body via `Body::Bytes` before sending.",
                    ));
                }
                return Self::digest_followup(
                    &session, attempt, auth, challenge, &uri_path, replay, deadline,
                )
                .await;
            }

            let sleep =
                match plan_retry(&result, &retry_policy, n, retryable_method, body_retryable) {
                    RetryPlan::Stop => return result,
                    RetryPlan::Backoff(sleep) => sleep,
                };

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::time::sleep(sleep.min(remaining)).await;
            n += 1;
            attempt.body = replay.clone().map(Body::Bytes).unwrap_or(Body::Empty);
        }
    }

    fn into_attempt(mut self) -> Attempt {
        let headers = if self.headers.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.headers))
        };
        Attempt {
            method: std::mem::take(&mut self.method),
            url: std::mem::take(&mut self.url),
            preset: self.preset,
            body: std::mem::take(&mut self.body),
            headers,
            timeout: self.timeout,
            timeouts: self.timeouts,
            stream_response: self.stream_response,
            proxy: self.proxy.take(),
            header_order: self.header_order.take(),
        }
    }

    pub(crate) fn prepare(&mut self) -> Result<()> {
        if let Some(err) = self.builder_error.take() {
            return Err(err);
        }
        self.infer_from_content_type();

        if !self.query_params.is_empty() {
            let mut url = url::Url::parse(&self.url)?;
            {
                let mut pairs = url.query_pairs_mut();
                for (k, v) in &self.query_params {
                    pairs.append_pair(k, v);
                }
            }
            self.url = url.to_string();
        }

        if let Some(encoding) = self.compress {
            match &self.body {
                Body::Bytes(b) => {
                    let compressed = encoding.encode(b)?;
                    self.headers
                        .set("content-encoding", encoding.header_value())?;
                    self.body = Body::from(compressed);
                }
                Body::Empty => {}
                Body::Stream { .. } => {
                    return Err(Error::new(Kind::Body).with_message(
                        "request-body compression is not supported for streaming bodies; \
                         buffer the body via `Body::Bytes` before calling `.compress(..)`",
                    ));
                }
            }
        }
        Ok(())
    }

    async fn digest_followup(
        session: &crate::core::Session,
        attempt: Attempt,
        auth: &crate::core::digest::DigestAuth,
        challenge: crate::core::digest::Challenge,
        uri_path: &str,
        replay: Option<bytes::Bytes>,
        deadline: tokio::time::Instant,
    ) -> Result<Response> {
        let mut challenge = challenge;
        let mut stale_retried = false;
        loop {
            let cnonce = crate::core::digest::generate_cnonce();
            let nc = crate::core::digest::next_nc_for_nonce(&challenge.nonce);
            let auth_header = match crate::core::digest::build_auth_header(
                &challenge,
                auth,
                attempt.method.as_str(),
                uri_path,
                nc,
                &cnonce,
            ) {
                Some(h) => h,
                None => {
                    return Err(Error::new(Kind::Request).with_message(
                        "digest auth: server offered only qop=auth-int, \
                         which Leyline does not implement (RFC 7616 §3.4.3 \
                         requires the entity-body hash in HA2). Pass through \
                         the 401 or remove digest_auth().",
                    ));
                }
            };
            let mut digest_headers = attempt.headers.clone().unwrap_or_default();
            digest_headers.set("authorization", auth_header)?;
            let replay_body = replay.clone().map(Body::Bytes).unwrap_or(Body::Empty);
            let digest_remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if digest_remaining.is_zero() {
                return Err(Error::new(Kind::Timeout));
            }
            let resp = session
                .run(attempt.again(replay_body, Some(digest_headers), Some(digest_remaining)))
                .await?;
            if resp.status() == 401
                && !stale_retried
                && let Some(next) = resp
                    .header("www-authenticate")
                    .and_then(|v| crate::core::digest::parse_challenge(v).ok())
                && next.stale
            {
                tracing::debug!(
                    target: "leyline::digest",
                    nonce = %next.nonce,
                    "stale nonce — retrying with fresh challenge"
                );
                challenge = next;
                stale_retried = true;
                continue;
            }
            return Ok(resp);
        }
    }
}

enum RetryPlan {
    Stop,
    Backoff(std::time::Duration),
}

fn plan_retry(
    result: &Result<Response>,
    retry_policy: &crate::core::retry::RetryPolicy,
    attempt: u32,
    retryable_method: bool,
    body_retryable: bool,
) -> RetryPlan {
    if retry_policy.is_none() || attempt >= retry_policy.max_retries {
        return RetryPlan::Stop;
    }
    let should_retry = match result {
        Ok(resp) => retry_policy.matches_status(resp.status().as_u16()),
        Err(err) if err.is_timeout() => retry_policy.matches_timeout(),
        Err(err) if err.is_connect() || err.is_connection_closed() => {
            retry_policy.matches_connection_error()
        }
        Err(_) => false,
    };
    if !should_retry || !retryable_method || !body_retryable {
        return RetryPlan::Stop;
    }
    let retry_after = match result {
        Ok(resp) => resp
            .header("retry-after")
            .and_then(crate::core::retry::parse_retry_after),
        _ => None,
    };
    match retry_after {
        Some(wait) if wait > retry_policy.max_retry_after => RetryPlan::Stop,
        Some(wait) => RetryPlan::Backoff(wait),
        None => RetryPlan::Backoff(retry_policy.backoff(attempt)),
    }
}

impl std::future::IntoFuture for RequestBuilder {
    type Output = Result<Response>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.send().await })
    }
}

#[cfg(test)]
#[path = "send_tests.rs"]
mod tests;
