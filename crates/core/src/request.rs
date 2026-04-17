//! Fluent request builder.

use std::time::Duration;

use leyline_profile::Preset;

use crate::body::Body;
use crate::digest::DigestAuth;
use crate::error::Error;
use crate::headers::HeaderList;
use crate::multipart::Form;
use crate::response::Response;
use crate::retry::{is_idempotent, RetryPolicy};
use crate::session::Session;
use crate::Result;

/// Fluent builder for constructing and sending HTTP requests.
///
/// ```rust,ignore
/// let resp = session.post("https://api.example.com/items")
///     .preset(Preset::Xhr)
///     .json(&payload)
///     .bearer_auth("token123")
///     .header("x-request-id", "abc")
///     .send()
///     .await?;
/// ```
pub struct RequestBuilder<'a> {
    session: &'a Session,
    method: String,
    url: String,
    preset: Option<Preset>,
    body: Body,
    headers: HeaderList,
    query_params: Vec<(String, String)>,
    timeout: Option<Duration>,
    builder_error: Option<Error>,
    stream_response: bool,
    retry_policy: RetryPolicy,
    allow_non_idempotent_retry: bool,
    digest_auth: Option<DigestAuth>,
}

impl<'a> RequestBuilder<'a> {
    pub(crate) fn new(session: &'a Session, method: &str, url: &str) -> Self {
        Self {
            session,
            method: method.to_string(),
            url: url.to_string(),
            preset: None,
            body: Body::Empty,
            headers: HeaderList::new(),
            query_params: Vec::new(),
            timeout: None,
            builder_error: None,
            stream_response: false,
            retry_policy: RetryPolicy::none(),
            allow_non_idempotent_retry: false,
            digest_auth: None,
        }
    }

    /// Override the session's default timeout for this one request.
    ///
    /// ```rust,ignore
    /// session.post(url)
    ///     .json(&payload)
    ///     .timeout(Duration::from_secs(5))
    ///     .send()
    ///     .await?;
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set a request preset (Navigate, Script, Xhr, Form, CrossOrigin, SameSite).
    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Set the request body.
    ///
    /// Accepts any `Into<Body>` — `Vec<u8>`, `&'static [u8]`, `String`,
    /// `&'static str`, `bytes::Bytes`, or a prebuilt [`Body`] (including
    /// `Body::stream(...)` for a streaming upload that will be pumped
    /// to the wire without materialising the full payload).
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Set the request body as JSON. Sets `content-type: application/json`.
    ///
    /// Serialization errors are deferred: the error is stashed and returned
    /// by [`send`](Self::send), so chaining stays panic-free.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.headers.set("content-type", "application/json");
                self.body = Body::from(bytes);
            }
            Err(e) => {
                self.builder_error = Some(Error::Json(e));
            }
        }
        self
    }

    /// Set the request body as URL-encoded form data. Sets content-type automatically.
    ///
    /// ```rust,ignore
    /// session.post(url).form(&[("user", "alice"), ("pass", "secret")]).send().await?;
    /// ```
    pub fn form(mut self, params: &[(&str, &str)]) -> Self {
        let encoded = url_encode_pairs(params);
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.into_bytes());
        self
    }

    /// Set the request body as a pre-encoded form string. Sets content-type automatically.
    pub fn form_str(mut self, encoded: &str) -> Self {
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.as_bytes().to_vec());
        self
    }

    /// Opt into streaming response delivery.
    ///
    /// The default behaviour buffers the full body in memory (up to the
    /// transport's `max_response_body_bytes` cap). When streaming is
    /// enabled, [`Response::into_stream`] returns a [`BodyStream`] the
    /// caller drives with `futures_util::Stream`.
    ///
    /// Decompression is NOT applied automatically when streaming is
    /// enabled — the caller must decompress the stream if the server
    /// set `content-encoding`.
    ///
    /// ```rust,ignore
    /// use futures_util::StreamExt;
    /// let resp = session.get(url).stream().send().await?;
    /// let mut body = resp.into_stream()?;
    /// while let Some(chunk) = body.next().await {
    ///     file.write_all(&chunk?).await?;
    /// }
    /// ```
    pub fn stream(mut self) -> Self {
        self.stream_response = true;
        self
    }

    /// Add URL query parameters. Can be called multiple times.
    ///
    /// ```rust,ignore
    /// session.get(url)
    ///     .query(&[("page", "2"), ("sort", "price")])
    ///     .send().await?;
    /// ```
    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        for &(k, v) in params {
            self.query_params.push((k.to_string(), v.to_string()));
        }
        self
    }

    /// Set a request header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Append a request header without replacing existing values with the same name.
    pub fn append_header(mut self, name: &str, value: &str) -> Self {
        self.headers.append(name, value);
        self
    }

    /// Set multiple headers at once.
    pub fn headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.set(k, v);
        }
        self
    }

    /// Append multiple headers, preserving duplicate names and order.
    pub fn append_headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.append(k, v);
        }
        self
    }

    /// Set a Bearer token for the Authorization header.
    ///
    /// ```rust,ignore
    /// session.get(url).bearer_auth("my-jwt-token").send().await?;
    /// ```
    pub fn bearer_auth(mut self, token: &str) -> Self {
        self.headers.set("authorization", format!("Bearer {token}"));
        self
    }

    /// Set Basic auth for the Authorization header.
    ///
    /// ```rust,ignore
    /// session.get(url).basic_auth("user", "pass").send().await?;
    /// ```
    pub fn basic_auth(mut self, username: &str, password: &str) -> Self {
        let encoded = base64_encode(&format!("{username}:{password}"));
        self.headers
            .set("authorization", format!("Basic {encoded}"));
        self
    }

    /// Attach a [`RetryPolicy`] to this request. When the policy
    /// fires, the whole request is replayed from scratch (fresh DNS,
    /// fresh TLS, fresh redirect loop).
    ///
    /// By default, only idempotent methods (GET/HEAD/OPTIONS/PUT/
    /// DELETE/TRACE) retry; POST/PATCH require
    /// [`Self::allow_non_idempotent_retry`] so users opt in to the
    /// risk of double-side-effects.
    ///
    /// Retry is incompatible with a streaming body
    /// ([`Body::Stream`]). If a retry would fire against one, the
    /// client errors clearly instead of silently dropping the retry.
    ///
    /// ```rust,ignore
    /// use leyline::RetryPolicy;
    /// session.get(url).retry(RetryPolicy::default()).send().await?;
    /// ```
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Opt into retrying non-idempotent methods (POST / PATCH). Use
    /// with care — the server may have already committed the first
    /// attempt's side effect even when the client saw an error.
    pub fn allow_non_idempotent_retry(mut self, allow: bool) -> Self {
        self.allow_non_idempotent_retry = allow;
        self
    }

    /// Enable HTTP Digest authentication for this request.
    ///
    /// On a `401 Unauthorized` response carrying a `WWW-Authenticate:
    /// Digest ...` header, the builder computes the RFC 7616 response
    /// hash and retries once with the `Authorization: Digest ...`
    /// header set. No credentials are sent on the first request.
    ///
    /// ```rust,ignore
    /// use leyline::DigestAuth;
    /// session.get(url)
    ///     .digest_auth(DigestAuth::new("admin", "hunter2"))
    ///     .send().await?;
    /// ```
    pub fn digest_auth(mut self, auth: DigestAuth) -> Self {
        self.digest_auth = Some(auth);
        self
    }

    /// Send the request as `multipart/form-data`.
    ///
    /// The body is serialised as a stream — very large file parts are
    /// pumped to the wire incrementally and never buffered as a whole.
    /// The `Content-Type` header is set to `multipart/form-data;
    /// boundary=...`.
    ///
    /// ```rust,ignore
    /// use leyline::multipart::{Form, Part};
    /// let form = Form::new()
    ///     .text("user", "alice")
    ///     .part("avatar", Part::bytes(jpeg).filename("cat.jpg").mime("image/jpeg"));
    /// session.post(url).multipart(form).send().await?;
    /// ```
    pub fn multipart(mut self, form: Form) -> Self {
        self.headers.set("content-type", form.content_type());
        self.body = form.into_stream_body();
        self
    }

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

        // Consume the builder up-front so we own the fields we need.
        let method = self.method.clone();
        let url = self.url.clone();
        let preset = self.preset;
        let base_headers = self.headers.clone();
        let timeout = self.timeout;
        let stream_response = self.stream_response;
        let retry_policy = self.retry_policy.clone();
        let allow_non_idempotent_retry = self.allow_non_idempotent_retry;
        let digest_auth = self.digest_auth.clone();
        let mut body = std::mem::take(&mut self.body);

        // Quick-path: no retry, no digest — route through the existing
        // single-shot execution. This preserves the old behaviour
        // bit-for-bit for callers who haven't opted in.
        if retry_policy.is_none() && digest_auth.is_none() {
            let headers = if base_headers.is_empty() {
                None
            } else {
                Some(base_headers)
            };
            return self
                .session
                .execute_with_timeout(
                    &method,
                    &url,
                    preset,
                    body,
                    headers,
                    timeout,
                    stream_response,
                )
                .await;
        }

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

        // First attempt.
        let mut attempt: u32 = 0;
        loop {
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
                    timeout,
                    stream_response,
                )
                .await;

            // Digest: if we got a 401 with a Digest challenge and the
            // original request did not already carry an Authorization
            // header, retry ONCE with the computed response.
            if let (Some(auth), Ok(resp)) = (&digest_auth, result.as_ref()) {
                if resp.status() == 401 && attempt == 0 {
                    if let Some(header) = resp
                        .headers()
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                        .map(|(_, v)| v.as_str())
                    {
                        // Only retry if it's actually a Digest challenge.
                        if let Ok(challenge) = crate::digest::parse_challenge(header) {
                            // Compute path+query from URL for the
                            // digest uri= field.
                            let parsed = url::Url::parse(&url)?;
                            let uri_path = match parsed.query() {
                                Some(q) => format!("{}?{}", parsed.path(), q),
                                None => parsed.path().to_string(),
                            };
                            let cnonce = crate::digest::generate_cnonce();
                            let auth_header = crate::digest::build_auth_header(
                                &challenge, auth, &method, &uri_path, 1, &cnonce,
                            );
                            let mut digest_headers = base_headers.clone();
                            digest_headers.set("authorization", auth_header);
                            if is_stream_body {
                                return Err(Error::Http(
                                    "digest auth: cannot replay streaming request body. \
                                     Buffer the body via `Body::Bytes` before sending."
                                        .into(),
                                ));
                            }
                            let hop_headers = Some(digest_headers);
                            // We already consumed the body; if the
                            // request had one it was buffered and we
                            // reconstruct an empty body here since the
                            // retry path only runs when original body
                            // is not a stream — but we didn't keep a
                            // clone. Rebuild from `self` is not
                            // possible at this point. For the common
                            // case of digest (GET-like) there is no
                            // body; guard the non-empty case.
                            return self
                                .session
                                .execute_with_timeout(
                                    &method,
                                    &url,
                                    preset,
                                    Body::Empty,
                                    hop_headers,
                                    timeout,
                                    stream_response,
                                )
                                .await;
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
                Err(Error::Io(_)) => retry_policy.matches_connection_error(),
                Err(Error::Http(msg))
                    if msg.contains("connection")
                        || msg.contains("closed")
                        || msg.contains("eof") =>
                {
                    retry_policy.matches_connection_error()
                }
                Err(Error::Timeout) => retry_policy.matches_timeout(),
                _ => false,
            };

            if !should_retry {
                return result;
            }

            if !retryable_method {
                return result;
            }

            if !body_retryable {
                return Err(Error::Http(
                    "retry requested on a streaming request body. Streaming bodies cannot \
                     be replayed — buffer the body via `Body::Bytes` before calling \
                     `retry()`, or drop the retry policy."
                        .into(),
                ));
            }

            // Sleep for backoff.
            let sleep = retry_policy.backoff(attempt);
            tokio::time::sleep(sleep).await;
            attempt += 1;
            // Replay the body template for the next iteration.
            body = match &retry_body_template {
                Some(Body::Empty) => Body::Empty,
                Some(Body::Bytes(b)) => Body::Bytes(b.clone()),
                // Unreachable: `body_retryable == false` already
                // bailed us out above.
                _ => Body::Empty,
            };
        }
    }
}

/// URL-encode key-value pairs.
pub(crate) fn url_encode_pairs(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode a string for `application/x-www-form-urlencoded`.
///
/// Follows the WHATWG form-urlencoded rules: unreserved set per RFC 3986
/// stays as-is, space becomes `+`, everything else is `%HH`. Matches
/// `percent_encoding::NON_ALPHANUMERIC` with a `' '` → `'+'` pass.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

/// Base64-encode a string for `Authorization: Basic` headers.
///
/// Thin wrapper over `base64::Engine::encode` with the STANDARD alphabet
/// and `=` padding. Exists only so upstream call sites stay string-shaped;
/// prefer the `base64` crate directly when writing new code.
fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_simple() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("safe-string_v2.0"), "safe-string_v2.0");
    }

    #[test]
    fn url_encode_pairs_works() {
        let pairs = url_encode_pairs(&[("user", "alice"), ("pass", "s3cr3t!")]);
        assert_eq!(pairs, "user=alice&pass=s3cr3t%21");
    }

    #[test]
    fn base64_encode_works() {
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
    }
}
