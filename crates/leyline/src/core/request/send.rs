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
        if let Some(err) = self.builder_error {
            return Err(err);
        }

        // Append query params to URL.
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

        // Request-body compression (opt-in via `.compress(..)`). Compress the
        // buffered body and declare `Content-Encoding` before the headers and
        // body are read below, so both the quick-path and retry-path see it.
        if let Some(encoding) = self.compress {
            match &self.body {
                Body::Bytes(b) => {
                    let compressed = encoding.encode(b)?;
                    self.headers
                        .set("content-encoding", encoding.header_value());
                    // The body length changes; content-length is recomputed
                    // authoritatively from the compressed bytes in `execute`,
                    // which strips any caller-supplied content-length first.
                    self.body = Body::from(compressed);
                }
                // An empty body has nothing to compress; emit no header.
                Body::Empty => {}
                // A streaming body cannot be compressed in place — refuse
                // rather than send raw bytes under a compressed header.
                Body::Stream { .. } => {
                    return Err(Error::Body(
                        "request-body compression is not supported for streaming bodies; \
                         buffer the body via `Body::Bytes` before calling `.compress(..)`"
                            .into(),
                    ));
                }
            }
        }

        // Quick-path: no retry, no digest — the bit-for-bit behaviour for
        // callers who have not opted in. Move/borrow straight out of `self`
        // so the hot path clones neither the method, the URL, nor the header
        // list (the retry path below still clones, as it must replay them).
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

        // Retry / digest path — clone the fields we must replay across attempts.
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

        // Retry / digest path. Both are request-level concerns: retry
        // re-runs the full `execute_inner`, and digest needs one extra
        // shot after parsing the challenge. We handle them together.
        let retryable_method = allow_non_idempotent_retry || is_idempotent(&method);
        let body_retryable = !body.is_stream();

        // For the retry path we need to be able to replay the body
        // across attempts. `Body::Bytes` is cheaply cloneable
        // (ref-counted `bytes::Bytes`); `Body::Stream` is not —
        // attempting a retry on a stream falls out as a clear error
        // below. Capture the retry-time body template here.
        let retry_body_template: Option<Body> = match &body {
            Body::Empty => Some(Body::Empty),
            Body::Bytes(b) => Some(Body::Bytes(b.clone())),
            Body::Stream { .. } => None,
        };

        // Overall-timeout budget: the caller's `.timeout(duration)` (or
        // the session default) caps the ENTIRE retry + digest loop, not
        // each attempt in isolation.
        let session_timeout = timeout.unwrap_or_else(|| self.session.default_timeout());
        let deadline = tokio::time::Instant::now() + session_timeout;

        // First attempt.
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

            // Digest: if we got a 401 with a Digest challenge and the
            // original request did not already carry an Authorization
            // header, retry ONCE with the computed response.
            if let (Some(auth), Ok(resp)) = (&digest_auth, result.as_ref()) {
                if resp.status() == 401 && attempt == 0 {
                    if let Some(header) = resp.header("www-authenticate") {
                        if let Ok(challenge) = crate::core::digest::parse_challenge(header) {
                            tracing::debug!(
                                target: "leyline::digest",
                                realm = %challenge.realm,
                                algorithm = ?challenge.algorithm,
                                stale = challenge.stale,
                                "digest 401 — computing response and retrying"
                            );
                            // The 401 challenge may originate from a URL other
                            // than the caller's original: `execute_with_timeout`
                            // follows redirects internally. HA2 = H(method:uri)
                            // must use the request-target the server actually
                            // challenged — the response's final URL — not the
                            // original. A same-origin redirect before the 401
                            // then still yields a valid digest: the authenticated
                            // retry re-follows that redirect carrying the
                            // authorization header, and the server validates HA2
                            // against the same `/protected` target we signed.
                            //
                            // Limitation: a cross-origin redirect strips the
                            // authorization header on the follow (see
                            // `execute_inner`'s sensitive-header stripping), so
                            // digest across a cross-origin redirect cannot
                            // complete — the retry lands unauthenticated and the
                            // 401 passes through.
                            let parsed = url::Url::parse(resp.url())?;
                            let uri_path = match parsed.query() {
                                Some(q) => format!("{}?{}", parsed.path(), q),
                                None => parsed.path().to_string(),
                            };
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
                                    &challenge, auth, &method, &uri_path, nc, &cnonce,
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
                                let replay_body = match &retry_body_template {
                                    Some(Body::Empty) => Body::Empty,
                                    Some(Body::Bytes(b)) => Body::Bytes(b.clone()),
                                    _ => Body::Empty,
                                };
                                let digest_remaining =
                                    deadline.saturating_duration_since(tokio::time::Instant::now());
                                if digest_remaining.is_zero() {
                                    return Err(Error::Timeout);
                                }
                                let resp = self
                                    .session
                                    .execute_with_timeout(
                                        &method,
                                        &url,
                                        preset,
                                        replay_body,
                                        Some(digest_headers),
                                        Some(digest_remaining),
                                        stream_response,
                                        request_proxy.as_deref(),
                                        self.header_order.as_deref(),
                                    )
                                    .await?;
                                // RFC 7616 §3.3: stale=true means the
                                // credentials were accepted but the nonce
                                // expired — retry once with the fresh nonce,
                                // never re-prompting. Single-retry cap so a
                                // hostile server cannot loop us.
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
                }
            }

            // Retry decision.
            if retry_policy.is_none() || attempt >= retry_policy.max_retries {
                return result;
            }

            let should_retry = match &result {
                Ok(resp) => retry_policy.matches_status(resp.status()),
                // Transport-level connection loss is typed: a mid-exchange H1
                // EOF surfaces as Io(UnexpectedEof) (see h1_error_to_core), so
                // the typed Io arm covers it. `Error::Http(_)` framing/protocol
                // errors are deliberately NOT retried — their message can carry
                // attacker-controlled header bytes, so substring-matching it
                // into a retry was a request-smuggling hazard.
                Err(Error::Io(_)) => retry_policy.matches_connection_error(),
                Err(Error::Timeout) => retry_policy.matches_timeout(),
                Err(Error::Tls(err)) if err.is_retryable() => {
                    retry_policy.matches_connection_error()
                }
                // Safely-retryable HTTP/2 transport failures: a transport-level
                // IO error, a graceful GOAWAY (NoError), or a server-side
                // REFUSED_STREAM (the request never began processing). Other
                // H2 errors (FrameTooLarge, ProtocolError, …) are NOT retried.
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

            if !should_retry {
                return result;
            }

            if !retryable_method {
                return result;
            }

            if !body_retryable {
                // The request hit a retryable error, but its streaming body
                // can't be replayed, so the retry can't happen. Surface a clear
                return Err(Error::Http(
                    "the request hit a retryable failure, but its streaming request body \
                     cannot be replayed. Buffer the body via `Body::Bytes` before retrying."
                        .into(),
                ));
            }

            // A server-directed `Retry-After` (delta-seconds) overrides our own
            // exponential backoff; bound it by the overall deadline so a hostile
            // or absent value can never exceed the caller's timeout.
            let sleep = match &result {
                Ok(resp) => resp
                    .header("retry-after")
                    .and_then(crate::core::retry::parse_retry_after),
                _ => None,
            }
            .unwrap_or_else(|| retry_policy.backoff(attempt));
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
}
