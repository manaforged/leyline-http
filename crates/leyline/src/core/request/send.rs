//! Request dispatch — quick-path, digest auth, and retry loop.

use super::RequestBuilder;
use crate::core::Result;
use crate::core::body::Body;
use crate::core::error::Error;
use crate::core::response::Response;
use crate::core::retry::is_idempotent;

impl RequestBuilder {
    /// Send the request and return a buffered response.
    pub async fn send(mut self) -> Result<Response> {
        self.prepare()?;
        if self.retry_policy.is_none() && self.digest_auth.is_none() {
            let body = std::mem::take(&mut self.body);
            let request_proxy = self.proxy.take();
            let headers = if self.headers.is_empty() {
                None
            } else {
                Some(std::mem::take(&mut self.headers))
            };
            return self
                .session
                .execute_with_timeout(
                    &self.method,
                    &self.url,
                    self.preset,
                    body,
                    headers,
                    self.timeout,
                    self.stream_response,
                    request_proxy.as_deref(),
                    self.header_order.as_deref(),
                )
                .await;
        }

        let method = self.method.clone();
        let url = self.url.clone();
        let preset = self.preset;
        let base_headers = self.headers.clone();
        let timeout = self.timeout;
        let stream_response = self.stream_response;
        let retry_policy = self.retry_policy.clone();
        let allow_non_idempotent_retry = self.allow_non_idempotent_retry;
        let digest_auth = self.digest_auth.clone();
        let request_proxy = self.proxy.take();
        let mut body = std::mem::take(&mut self.body);

        let retryable_method = allow_non_idempotent_retry || is_idempotent(&method);
        let body_retryable = !body.is_stream();

        let retry_body_template: Option<Body> = match &body {
            Body::Empty => Some(Body::Empty),
            Body::Bytes(b) => Some(Body::Bytes(b.clone())),
            Body::Stream { .. } => None,
        };

        let session_timeout = timeout.unwrap_or_else(|| self.session.default_timeout());
        let deadline = tokio::time::Instant::now() + session_timeout;

        let mut attempt: u32 = 0;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let attempt_timeout = Some(remaining);
            let this_headers = if base_headers.is_empty() {
                None
            } else {
                Some(base_headers.clone())
            };
            let hop_body = std::mem::take(&mut body);
            let is_stream_body = hop_body.is_stream();

            let result = self
                .session
                .execute_with_timeout(
                    &method,
                    &url,
                    preset,
                    hop_body,
                    this_headers,
                    attempt_timeout,
                    stream_response,
                    request_proxy.as_deref(),
                    self.header_order.as_deref(),
                )
                .await;

            if let (Some(auth), Ok(resp)) = (&digest_auth, result.as_ref()) {
                if resp.status() == 401 && attempt == 0 {
                    if let Some(header) = resp.header("www-authenticate") {
                        if let Ok(challenge) = crate::core::digest::parse_challenge(header) {
                            let parsed = url::Url::parse(resp.url())?;
                            let uri_path = match parsed.query() {
                                Some(q) => format!("{}?{}", parsed.path(), q),
                                None => parsed.path().to_string(),
                            };
                            return Self::digest_followup(
                                &self.session,
                                self.header_order.as_deref(),
                                auth,
                                challenge,
                                &url,
                                &method,
                                &uri_path,
                                preset,
                                &base_headers,
                                match retry_body_template.as_ref() {
                                    Some(Body::Bytes(b)) => Body::Bytes(b.clone()),
                                    _ => Body::Empty,
                                },
                                deadline,
                                stream_response,
                                request_proxy.as_deref(),
                                is_stream_body,
                            )
                            .await;
                        }
                    }
                }
            }

            let sleep = match plan_retry(
                &result,
                &retry_policy,
                attempt,
                retryable_method,
                body_retryable,
            ) {
                RetryPlan::Stop => return result,
                RetryPlan::Abort(e) => return Err(e),
                RetryPlan::Backoff(sleep) => sleep,
            };
            if !retryable_method {
                return result;
            }

            if !body_retryable {
                return Err(Error::Http(
                    "the request hit a retryable failure, but its streaming request body \
                     cannot be replayed. Buffer the body via `Body::Bytes` before retrying."
                        .into(),
                ));
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::time::sleep(sleep.min(remaining)).await;
            attempt += 1;
            body = match &retry_body_template {
                Some(Body::Empty) => Body::Empty,
                Some(Body::Bytes(b)) => Body::Bytes(b.clone()),
                _ => Body::Empty,
            };
        }
    }

    /// Builder-error replay, query-param append, and opt-in request-body compression.
    fn prepare(&mut self) -> Result<()> {
        if let Some(err) = self.builder_error.take() {
            return Err(err);
        }

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
                        .set("content-encoding", encoding.header_value());
                    self.body = Body::from(compressed);
                }
                Body::Empty => {}
                Body::Stream { .. } => {
                    return Err(Error::Body(
                        "request-body compression is not supported for streaming bodies; \
                         buffer the body via `Body::Bytes` before calling `.compress(..)`"
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Handle a 401 Digest challenge: compute the Authorization response and retry once, honoring stale=true with a single fresh-nonce retry (RFC 7616 §3.3).
    #[allow(clippy::too_many_arguments)]
    async fn digest_followup(
        session: &crate::core::Session,
        header_order: Option<&[String]>,
        auth: &crate::core::digest::DigestAuth,
        challenge: crate::core::digest::Challenge,
        url: &str,
        method: &str,
        uri_path: &str,
        preset: Option<crate::profile::Preset>,
        base_headers: &crate::core::headers::HeaderList,
        replay_template: Body,
        deadline: tokio::time::Instant,
        stream_response: bool,
        request_proxy: Option<&str>,
        is_stream_body: bool,
    ) -> Result<Response> {
        if is_stream_body {
            return Err(Error::Http(
                "digest auth: cannot replay streaming request body. \
                 Buffer the body via `Body::Bytes` before sending."
                    .into(),
            ));
        }
        let mut challenge = challenge;
        let mut stale_retried = false;
        loop {
            let cnonce = crate::core::digest::generate_cnonce();
            let nc = crate::core::digest::next_nc_for_nonce(&challenge.nonce);
            let auth_header = match crate::core::digest::build_auth_header(
                &challenge, auth, method, uri_path, nc, &cnonce,
            ) {
                Some(h) => h,
                None => {
                    return Err(Error::Http(
                        "digest auth: server offered only qop=auth-int, \
                         which Leyline does not implement (RFC 7616 §3.4.3 \
                         requires the entity-body hash in HA2). Pass through \
                         the 401 or remove digest_auth()."
                            .into(),
                    ));
                }
            };
            let mut digest_headers = base_headers.clone();
            digest_headers.set("authorization", auth_header);
            let replay_body = match &replay_template {
                Body::Bytes(b) => Body::Bytes(b.clone()),
                _ => Body::Empty,
            };
            let digest_remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if digest_remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let resp = session
                .execute_with_timeout(
                    method,
                    url,
                    preset,
                    replay_body,
                    Some(digest_headers),
                    Some(digest_remaining),
                    stream_response,
                    request_proxy,
                    header_order,
                )
                .await?;
            if resp.status() == 401 && !stale_retried {
                let next = resp
                    .header("www-authenticate")
                    .and_then(|v| crate::core::digest::parse_challenge(v).ok());
                if let Some(next) = next {
                    if next.stale {
                        tracing::debug!(
                            target: "leyline::digest",
                            nonce = %next.nonce,
                            "stale nonce — retrying with fresh challenge"
                        );
                        challenge = next;
                        stale_retried = true;
                        continue;
                    }
                }
            }
            return Ok(resp);
        }
    }
}

/// What the retry loop does after one attempt.
enum RetryPlan {
    /// Give up and surface the attempt's result.
    Stop,
    /// Give up with a synthesized error (unreplayable streaming body).
    Abort(Error),
    /// Sleep this long, then try again.
    Backoff(std::time::Duration),
}

/// Applies the retry policy to one attempt's outcome.
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
        Ok(resp) => retry_policy.matches_status(resp.status()),
        Err(Error::Io(_)) => retry_policy.matches_connection_error(),
        Err(Error::Timeout) => retry_policy.matches_timeout(),
        Err(Error::Tls(err)) if err.is_retryable() => retry_policy.matches_connection_error(),
        Err(Error::Http2(
            crate::h2::H2Error::Io(_)
            | crate::h2::H2Error::Connection {
                code: crate::h2::error::ErrorCode::NoError,
                ..
            }
            | crate::h2::H2Error::Stream {
                code: crate::h2::error::ErrorCode::RefusedStream,
                ..
            },
        )) => retry_policy.matches_connection_error(),
        _ => false,
    };
    if !should_retry || !retryable_method {
        return RetryPlan::Stop;
    }
    if !body_retryable {
        return RetryPlan::Abort(Error::Http(
            "the request hit a retryable failure, but its streaming request body \
             cannot be replayed. Buffer the body via `Body::Bytes` before retrying."
                .into(),
        ));
    }
    let sleep = match result {
        Ok(resp) => resp
            .header("retry-after")
            .and_then(crate::core::retry::parse_retry_after),
        _ => None,
    }
    .unwrap_or_else(|| retry_policy.backoff(attempt));
    RetryPlan::Backoff(sleep)
}

impl std::future::IntoFuture for RequestBuilder {
    type Output = Result<Response>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.send().await })
    }
}
