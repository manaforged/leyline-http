use super::RequestBuilder;
use super::route::Route;
use crate::core::Result;
use crate::core::body::{Body, BodyKind};
use crate::core::deadline::Deadline;
use crate::core::error::{Error, Kind};
use crate::core::response::Response;
use crate::core::session::execute::Attempt;
use crate::util::is_idempotent;

const STATUS_ERROR_BODY_LIMIT: usize = 64 * 1024;

impl RequestBuilder {
    pub async fn send(mut self) -> Result<Response> {
        self.prepare()?;
        let retry_policy = self.retry_policy.clone();
        let session = self.session.clone();
        let deadline = session.deadline(self.timeouts.as_ref());
        let tag = self.tag.take();
        let status_errors = self.status_errors;
        let attempt = self.into_attempt(deadline);
        let method = attempt.method.clone();
        let url = attempt.url.clone();
        let streamed = attempt.stream_response;
        let mut exhausted = false;
        let response = session
            .traced(
                method.as_str(),
                &url,
                streamed,
                run_attempts(
                    &session,
                    attempt,
                    &retry_policy,
                    deadline,
                    tag,
                    &mut exhausted,
                ),
            )
            .await?;
        if !status_errors {
            return Ok(response);
        }
        let policy_wait = retry_policy.server_wait(&response);
        response
            .error_for_status_with_body(STATUS_ERROR_BODY_LIMIT, &deadline)
            .await
            .map_err(|mut error| {
                error.set_retry_outcome(policy_wait, exhausted);
                error
            })
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
            initiator: self.initiator.take(),
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

async fn run_attempts(
    session: &crate::core::session::Session,
    mut attempt: Attempt,
    retry_policy: &crate::core::retry::RetryPolicy,
    deadline: Deadline,
    tag: Option<String>,
    exhausted: &mut bool,
) -> (Result<Response>, u32) {
    crate::trace::note_tag(tag);
    let mut route = Route::new(session, &attempt);
    if retry_policy.is_none() {
        return (counted(route.send(attempt, None).await, 1), 1);
    }
    let retryable_method =
        retry_policy.allow_non_idempotent || is_idempotent(attempt.method.as_str());
    let replay = attempt.body.replay();
    let body_retryable = replay.is_some();
    let base_headers = attempt.headers.clone();

    let mut n: u32 = 0;
    loop {
        if let Err(err) = deadline.check() {
            return (Err(err), n);
        }
        let attempt_body = std::mem::take(&mut attempt.body);
        let this = attempt.again(attempt_body, base_headers.clone());
        let result = route.send(this, retry_policy.proxy_for_retry(n)).await;

        let sleep = match plan_retry(&result, retry_policy, n, retryable_method, body_retryable) {
            RetryPlan::Backoff(sleep) if sleep < deadline.remaining() => sleep,
            plan => {
                *exhausted = matches!(plan, RetryPlan::Exhausted);
                return (counted(result, n + 1), n + 1);
            }
        };

        let slept = session
            .unless_shut_down(async {
                deadline.sleep(sleep).await;
                Ok(())
            })
            .await;
        if let Err(err) = slept {
            return (counted(Err(err), n + 1), n + 1);
        }
        n += 1;
        attempt.body = replay.as_ref().and_then(Body::replay).unwrap_or_default();
    }
}

fn counted(result: Result<Response>, attempts: u32) -> Result<Response> {
    result
        .map(|mut response| {
            response.set_attempts(attempts);
            response
        })
        .map_err(|mut error| {
            error.set_attempts(attempts);
            error
        })
}

enum RetryPlan {
    Stop,
    Exhausted,
    Backoff(std::time::Duration),
}

fn plan_retry(
    result: &Result<Response>,
    retry_policy: &crate::core::retry::RetryPolicy,
    attempt: u32,
    retryable_method: bool,
    body_retryable: bool,
) -> RetryPlan {
    if retry_policy.is_none() {
        return RetryPlan::Stop;
    }
    if attempt >= retry_policy.max_retries {
        return match result {
            Ok(response) if retry_policy.matches_response(response) => RetryPlan::Exhausted,
            _ => RetryPlan::Stop,
        };
    }
    if !retry_wanted(result, retry_policy)
        || !(retryable_method || retry_policy.retries_unsent(result))
        || !body_retryable
    {
        return RetryPlan::Stop;
    }
    match result
        .as_ref()
        .ok()
        .and_then(|r| retry_policy.server_wait(r))
    {
        Some(wait) if wait > retry_policy.max_retry_after => RetryPlan::Stop,
        Some(wait) => RetryPlan::Backoff(wait),
        None => RetryPlan::Backoff(retry_policy.backoff(attempt)),
    }
}

fn retry_wanted(result: &Result<Response>, retry_policy: &crate::core::retry::RetryPolicy) -> bool {
    match result {
        Ok(resp) => retry_policy.matches_response(resp),
        Err(err) if err.is_timeout() => retry_policy.matches_timeout(),
        Err(err) if err.is_retryable() => retry_policy.matches_connection_error(),
        Err(_) => false,
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
