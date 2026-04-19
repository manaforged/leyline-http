use std::collections::HashMap;

use crate::profile::Preset;

use super::decompress::{decompress_body, drain_stream_into_vec};
use super::header_merge::apply_extra_headers;
use super::Session;
use crate::core::body::Body;
use crate::core::error::{Error, Result};
use crate::core::headers::HeaderList;
use crate::core::response::Response;

impl Session {
    // â”€â”€â”€ Core execution â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Execute a request with an optional per-request timeout override.
    /// When `override_timeout` is `None`, the session-level timeout applies.
    /// Handles redirects, decompression, cookies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_with_timeout(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        override_timeout: Option<std::time::Duration>,
        stream_response: bool,
    ) -> Result<Response> {
        let timeout = override_timeout.unwrap_or(self.timeout);
        // Box the inner future to move its state to the heap. Without this,
        // the combined RequestBuilder â†’ execute_with_timeout â†’ execute_inner
        // state machine is large enough to blow the default 2 MB thread
        // stack when a test awaits two requests sequentially.
        let inner: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(
            method,
            raw_url,
            preset,
            body,
            extra_headers,
            stream_response,
        ));
        match tokio::time::timeout(timeout, inner).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        }
    }

    #[tracing::instrument(
        name = "session.execute",
        level = "debug",
        skip_all,
        fields(http.method = method, http.url = raw_url)
    )]
    async fn execute_inner(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        stream_response: bool,
    ) -> Result<Response> {
        let mut current_url = url::Url::parse(raw_url)?;
        let original_origin = url_origin(&current_url);
        let mut current_method = method.to_string();
        // Carry the body through the redirect loop. A streaming body is
        // placed in `current_body` for the first hop; on a method- or
        // body-preserving redirect (307/308) we cannot replay a stream,
        // so a cross-redirect stream becomes an explicit error. Buffered
        // bodies replay fine because `Body::Bytes` is `Clone`-like.
        let mut current_body = body;
        let mut redirect_chain = Vec::new();
        let mut all_cookies = HashMap::new();

        for _ in 0..=self.max_redirects {
            let origin = url_origin(&current_url);
            let referer = if redirect_chain.is_empty() {
                format!("{}/", origin)
            } else {
                redirect_chain
                    .last()
                    .cloned()
                    .unwrap_or_else(|| format!("{}/", origin))
            };

            // Build headers.
            let mut headers: Vec<(String, String)> = if let Some(preset) = preset {
                let ctx = crate::profile::preset::HeaderContext {
                    user_agent: &self.user_agent,
                    sec_ch_ua: &self.sec_ch_ua,
                    sec_ch_ua_mobile: self.platform.mobile_flag(),
                    sec_ch_ua_platform: self.platform.sec_ch_platform(),
                    accept_language: &self.accept_language,
                    origin: &origin,
                    referer: &referer,
                };
                preset.build_headers(&ctx)
            } else {
                vec![
                    ("user-agent".into(), self.user_agent.clone()),
                    ("accept".into(), "*/*".to_string()),
                    (
                        "accept-encoding".into(),
                        "gzip, deflate, br, zstd".to_string(),
                    ),
                    ("accept-language".into(), self.accept_language.clone()),
                ]
            };

            // Apply Chromium-sibling overlays: Brave's trimmed
            // Navigate `accept`, and per-brand extra headers (Edge's
            // `dnt: 1`, Brave's `sec-gpc: 1`). These only run when
            // the builder's `.brand(..)` was set to a non-Chrome
            // variant AND the active profile is Chromium — both
            // gates were applied at build time, so here we only
            // need to splice the cached values in.
            if let (Some(accept_override), Some(Preset::Navigate)) =
                (self.brand_navigate_accept.as_deref(), preset)
            {
                for (name, value) in headers.iter_mut() {
                    if name == "accept" {
                        *value = accept_override.to_string();
                        break;
                    }
                }
            }
            // User's `.header(..)` / `.append_header(..)` always
            // wins over a brand's default — e.g. a caller on an
            // Edge session setting `.header("dnt", "0")` must not
            // see both `dnt: 1` (brand) and `dnt: 0` (user) on the
            // wire. We also skip any name that the active preset
            // already emitted, so a future preset shipping `dnt` or
            // `sec-gpc` by default doesn't collide with the overlay.
            for (k, v) in &self.brand_extra_headers {
                let user_has_it = extra_headers
                    .as_ref()
                    .map(|h| h.iter().any(|(uk, _)| uk.eq_ignore_ascii_case(k)))
                    .unwrap_or(false);
                let preset_has_it = headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k));
                if !user_has_it && !preset_has_it {
                    headers.push((k.clone(), v.clone()));
                }
            }

            // Extra headers: include on first request, and on same-origin redirects.
            // Strip sensitive headers on cross-origin redirects.
            //
            // Caller headers are merged into the preset-built list via
            // three rules:
            //   (1) A name the preset already emits is replaced in
            //       place — preserves the preset position and prevents
            //       wire-coalesced `ua1,ua2` duplicates (HTTP/1.1
            //       §3.2.2).
            //   (2) A caller-anchored header (`.anchored(anchor, ...)`)
            //       is spliced immediately after (or before, for
            //       `BeforeAcceptEncoding`) its anchor header in the
            //       current list.
            //   (3) A plain caller header (`.header(...)`) whose name
            //       has a universal Chrome slot per `infer_anchor`
            //       rides at the inferred anchor; otherwise it
            //       appends at the end of the preset list.
            if let Some(ref extra) = extra_headers {
                let same_origin = origin == original_origin;
                let strip_sensitive = !redirect_chain.is_empty() && !same_origin;
                let sensitive = |name: &str| {
                    let lower = name.to_ascii_lowercase();
                    lower == "authorization" || lower == "proxy-authorization" || lower == "cookie"
                };

                apply_extra_headers(&mut headers, extra, strip_sensitive, &sensitive);
            }

            // Content-Length for requests with a known-length body.
            // For length-unknown streams we leave it out and let the
            // transport pick `Transfer-Encoding: chunked` (H1) or native
            // framing (H2/H3).
            if let Some(len) = current_body.len_hint() {
                if !matches!(current_body, Body::Empty) || len > 0 {
                    headers.push(("content-length".into(), len.to_string()));
                }
            }

            // Cookies.
            if let Some(cookie_val) = self.cookie_jar.cookie_header(&current_url) {
                headers.push(("cookie".into(), cookie_val));
            }

            let audit_headers = headers.clone();

            // Take the body for this hop. Streams are one-shot; we replace
            // `current_body` with `Body::Empty` so a follow-up redirect
            // sees there's nothing to replay (and fails loudly).
            let hop_body = std::mem::take(&mut current_body);
            let hop_body_was_stream = hop_body.is_stream();

            // Send via the configured protocol policy.
            let transport_resp = self
                .send_with_policy(
                    &current_method,
                    &current_url,
                    headers,
                    hop_body,
                    stream_response,
                )
                .await?;
            let status = transport_resp.status;
            let resp_headers = transport_resp.headers;
            let resp_body_shape = transport_resp.body;
            let final_url = transport_resp.final_url;
            let response_version = transport_resp.version;
            let tls_alpn = transport_resp.tls_alpn;
            let peer_cert_der = transport_resp.peer_cert_der;
            let tls_version = transport_resp.tls_version;
            let tls_cipher = transport_resp.tls_cipher;

            // Store cookies from response and accumulate across redirect chain.
            let set_cookies: Vec<&str> = resp_headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                .map(|(_, v)| v.as_str())
                .collect();
            if !set_cookies.is_empty() {
                self.cookie_jar
                    .store_response_cookies(&set_cookies, &current_url);
                for sc in &set_cookies {
                    if let Some(eq) = sc.find('=') {
                        let name = sc[..eq].trim();
                        let rest = &sc[eq + 1..];
                        let value = rest.split(';').next().unwrap_or("").trim();
                        all_cookies.insert(name.to_string(), value.to_string());
                    }
                }
            }

            // Check for redirect.
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if let Some(location) = resp_headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v.clone())
                {
                    // Drain and discard the intermediate response body.
                    drop(resp_body_shape);
                    redirect_chain.push(current_url.to_string());
                    current_url = current_url.join(&location)?;

                    // 301/302/303: switch to GET, drop body.
                    // 307/308: preserve method and body. A streaming
                    // request body cannot be replayed — fail clearly.
                    if matches!(status, 301..=303) {
                        current_method = "GET".to_string();
                        current_body = Body::Empty;
                    } else if hop_body_was_stream {
                        return Err(Error::Http(format!(
                            "cannot follow {status} redirect: streaming request bodies are \
                             not replayable. Either buffer the body before sending or set \
                             max_redirects(0)."
                        )));
                    }
                    continue;
                }
            }

            // If the caller opted into streaming, deliver as-is WITHOUT
            // decompression. Otherwise materialise and decompress as today.
            let (final_body, final_headers) = match resp_body_shape {
                crate::core::transport::TransportBody::Streaming(bs) if stream_response => (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                ),
                crate::core::transport::TransportBody::Streaming(bs) => {
                    // Transport returned a stream but caller wanted
                    // buffering. Drain it fully here, then run normal
                    // decompression.
                    let buf = drain_stream_into_vec(bs).await?;
                    let content_encoding = resp_headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
                        .map(|(_, v)| v.to_lowercase());
                    let buf = decompress_body(buf, content_encoding.as_deref())?;
                    let resp_headers: Vec<(String, String)> = if content_encoding.is_some() {
                        resp_headers
                            .into_iter()
                            .filter(|(k, _)| {
                                !k.eq_ignore_ascii_case("content-encoding")
                                    && !k.eq_ignore_ascii_case("content-length")
                            })
                            .collect()
                    } else {
                        resp_headers
                    };
                    (
                        crate::core::response::ResponseBody::Buffered(buf),
                        resp_headers,
                    )
                }
                crate::core::transport::TransportBody::Buffered(buf) => {
                    if stream_response {
                        // Transport buffered (H1 / H3 path). Preserve
                        // content-encoding and hand the buffer over as a
                        // single-chunk stream so the API is uniform.
                        (
                            crate::core::response::ResponseBody::Streaming(
                                crate::core::body_stream::BodyStream::from_bytes(
                                    bytes::Bytes::from(buf),
                                ),
                            ),
                            resp_headers,
                        )
                    } else {
                        let content_encoding = resp_headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
                            .map(|(_, v)| v.to_lowercase());
                        let buf = decompress_body(buf, content_encoding.as_deref())?;
                        let resp_headers: Vec<(String, String)> = if content_encoding.is_some() {
                            resp_headers
                                .into_iter()
                                .filter(|(k, _)| {
                                    !k.eq_ignore_ascii_case("content-encoding")
                                        && !k.eq_ignore_ascii_case("content-length")
                                })
                                .collect()
                        } else {
                            resp_headers
                        };
                        (
                            crate::core::response::ResponseBody::Buffered(buf),
                            resp_headers,
                        )
                    }
                }
            };

            return Ok(Response {
                status,
                headers: final_headers,
                body: final_body,
                cookies: all_cookies,
                url: final_url,
                redirect_chain,
                version: response_version,
                trailers: Vec::new(),
                request_headers: audit_headers.clone(),
                tls_alpn,
                tls_peer_certificate: peer_cert_der,
                tls_version,
                tls_cipher,
                audit_data: Some(crate::audit::AuditData {
                    ja4: self.audit_tls.ja4.clone(),
                    ja3: self.audit_tls.ja3.clone(),
                    h2_fingerprint: self.audit_tls.h2_fingerprint.clone(),
                    ja4t: self.audit_tls.ja4t.clone(),
                    ja4h: {
                        let input = crate::audit::Ja4hInput {
                            method: &current_method,
                            http_version: response_version.ja4h_token(),
                            headers: &audit_headers,
                        };
                        crate::audit::compute_ja4h(&input)
                    },
                }),
            });
        }

        Err(Error::Http(format!(
            "too many redirects (max {})",
            self.max_redirects
        )))
    }
}

fn url_origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}
