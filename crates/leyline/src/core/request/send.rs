use super::RequestBuilder;
use crate::core::Result;
use crate::core::body::{Body, BodyKind};
use crate::core::deadline::Deadline;
use crate::core::error::{Error, Kind};
use crate::core::response::Response;
use crate::core::session::execute::Attempt;
use crate::util::is_idempotent;

impl RequestBuilder {
    pub async fn send(mut self) -> Result<Response> {
        self.prepare()?;
        let retry_policy = self.retry_policy.clone();
        let session = self.session.clone();
        let deadline = session.deadline(self.timeouts.as_ref());
        let mut attempt = self.into_attempt(deadline);

        if retry_policy.is_none() {
            return session.attempt(attempt).await;
        }

        let retryable_method =
            retry_policy.allow_non_idempotent || is_idempotent(attempt.method.as_str());
        let replay = attempt.body.replay();
        let body_retryable = replay.is_some();
        let base_headers = attempt.headers.clone();

        let mut n: u32 = 0;
        loop {
            deadline.check()?;
            let attempt_body = std::mem::take(&mut attempt.body);
            let this = attempt.again(attempt_body, base_headers.clone());
            let result = session.attempt(this).await;

            let sleep =
                match plan_retry(&result, &retry_policy, n, retryable_method, body_retryable) {
                    RetryPlan::Backoff(sleep) if sleep < deadline.remaining() => sleep,
                    _ => return result,
                };

            deadline.sleep(sleep).await;
            n += 1;
            attempt.body = replay.as_ref().and_then(Body::replay).unwrap_or_default();
        }
    }

    fn into_attempt(mut self, deadline: Deadline) -> Attempt {
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
            deadline,
            stream_response: self.stream_response,
            proxy: self.proxy.take(),
            header_order: self.header_order.take(),
            redirect: self.redirect.take(),
            digest: self.digest_auth.take(),
        }
    }

    pub(crate) fn prepare(&mut self) -> Result<()> {
        if let Some(err) = self.builder_error.take() {
            return Err(err);
        }
        self.infer_from_content_type();

        if !self.query_params.is_empty() {
            let mut url = url::Url::parse(&self.url).map_err(crate::core::Error::from_url_parse)?;
            {
                let mut pairs = url.query_pairs_mut();
                for (k, v) in &self.query_params {
                    pairs.append_pair(k, v);
                }
            }
            self.url = url.to_string();
        }

        if let Some(encoding) = self.compress {
            match &self.body.0 {
                BodyKind::Bytes(b) => {
                    let compressed = encoding.encode(b)?;
                    self.headers
                        .set("content-encoding", encoding.header_value())?;
                    self.body = Body::from(compressed);
                }
                BodyKind::Empty => {}
                BodyKind::Stream { .. } => {
                    return Err(Error::new(Kind::Body).with_message(
                        "request-body compression is not supported for streaming bodies; \
                         buffer the body into bytes before calling `.compress(..)`",
                    ));
                }
            }
        }
        Ok(())
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
        Err(err) if err.is_retryable() => retry_policy.matches_connection_error(),
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
        None => RetryPlan::Backoff(retry_policy.delay(attempt)),
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
