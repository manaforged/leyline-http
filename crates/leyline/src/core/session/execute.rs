use std::collections::HashMap;

use crate::profile::Preset;

use super::decompress::{decompress_body, drain_stream_into_vec};
use super::header_merge::apply_extra_headers;
use super::Session;
use crate::core::body::Body;
use crate::core::error::{Error, Result};
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::{RedirectAction, RedirectAttempt};

impl Session {
    // Core execution.

    /// Execute a request with an optional per-request timeout override.
    /// When `override_timeout` is `None`, the session-level timeout applies.
    /// `request_proxy`, when `Some`, overrides the session's default proxy
    /// for this single request (and any redirects it follows).
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
        request_proxy: Option<&str>,
    ) -> Result<Response> {
        let timeout = override_timeout.unwrap_or(self.timeouts.total);
        // Box the inner future to move its state to the heap. Without this,
        // the combined RequestBuilder -> execute_with_timeout -> execute_inner
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
            request_proxy,
        ));
        let result = match tokio::time::timeout(timeout, inner).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        };
        if let Err(ref e) = result {
            crate::observe::notify_request_error(&crate::observe::RequestErrorSnapshot {
                method,
                url: raw_url,
                error: &e.to_string(),
            });
        }
        result
    }

    #[tracing::instrument(
        name = "session.execute",
        level = "debug",
        skip_all,
        fields(http.method = method, http.url = raw_url)
    )]
    #[allow(clippy::too_many_arguments)]
    async fn execute_inner(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        stream_response: bool,
        request_proxy: Option<&str>,
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

        let redirect_cap = self.redirect_policy.max_redirects_hint();
        for _ in 0..=redirect_cap {
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

            // Apply Chromium-sibling brand overlays AND identity-level
            // overrides for first-class profiles. Both apply the same
            // shape of edits (Navigate `accept` swap, extra headers);
            // only one of the two paths fires for any given session
            // because brand overlays are off when Brave is first-class.
            let navigate_accept_override = self
                .identity_navigate_accept
                .as_deref()
                .or(self.brand_navigate_accept.as_deref());
            if let (Some(accept_override), Some(Preset::Navigate)) =
                (navigate_accept_override, preset)
            {
                for (name, value) in headers.iter_mut() {
                    if name == "accept" {
                        *value = accept_override.to_string();
                        break;
                    }
                }
            }
            // User's `.header(..)` / `.append_header(..)` always
            // wins over a brand or identity default — e.g. a caller on an
            // Edge session setting `.header("dnt", "0")` must not
            // see both `dnt: 1` (brand) and `dnt: 0` (user) on the
            // wire. We also skip any name that the active preset
            // already emitted, so a future preset shipping `dnt` or
            // `sec-gpc` by default doesn't collide with the overlay.
            for (k, v) in self
                .brand_extra_headers
                .iter()
                .chain(self.identity_extra_headers.iter())
            {
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

            // Identity-level header reordering. Browsers like Brave ship
            // a non-Chrome request-header sequence (e.g. accept-language
            // repositioned after sec-gpc, between accept and sec-fetch-*).
            // When the active identity declares a `request_header_order`,
            // sort the assembled headers to that order; names not in the
            // list keep their relative position at the tail.
            if let Some(order) = self.identity_request_header_order.as_deref() {
                reorder_headers(&mut headers, order);
            }

            let audit_headers = headers.clone();

            // Take the body for this hop. Streams are one-shot; we replace
            // `current_body` with `Body::Empty` so a follow-up redirect
            // sees there's nothing to replay (and fails loudly).
            let hop_body = std::mem::take(&mut current_body);
            let hop_body_was_stream = hop_body.is_stream();

            // Send via the configured protocol policy. The per-request
            // proxy override (if any) carries through every redirect
            // hop in this `execute_inner` call — once a caller picks a
            // proxy for a request, all redirects of that request go
            // through the same proxy.
            let transport_resp = self
                .send_with_policy(
                    &current_method,
                    &current_url,
                    headers,
                    hop_body,
                    stream_response,
                    request_proxy,
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
                    let action = self.redirect_policy.action(RedirectAttempt {
                        status,
                        url: &current_url,
                        location: Some(location.as_str()),
                        previous: &redirect_chain,
                    });
                    if action == RedirectAction::Stop {
                        // Fall through and return the redirect response as-is.
                    } else {
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
                            return Err(Error::Redirect(format!(
                                "cannot follow {status} redirect: streaming request bodies are \
                             not replayable. Either buffer the body before sending or set \
                             max_redirects(0)."
                            )));
                        }
                        continue;
                    }
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
                    let drain = drain_stream_into_vec(bs);
                    let buf = if let Some(read_timeout) = self.timeouts.read {
                        tokio::time::timeout(read_timeout, drain)
                            .await
                            .map_err(|_| Error::Timeout)??
                    } else {
                        drain.await?
                    };
                    let content_encoding = resp_headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
                        .map(|(_, v)| v.to_lowercase());
                    let (buf, decoded) =
                        decompress_body(buf, content_encoding.as_deref(), &self.compression)?;
                    let resp_headers: Vec<(String, String)> = if decoded {
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
                        let (buf, decoded) =
                            decompress_body(buf, content_encoding.as_deref(), &self.compression)?;
                        let resp_headers: Vec<(String, String)> = if decoded {
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

            // Fire the global response observer (if registered) before
            // handing the assembled response back to the caller. Streamed
            // responses pass an empty body slice — the observer cannot
            // drain a stream without changing semantics, and a dump-style
            // observer wouldn't get useful bytes anyway.
            {
                let body_slice: &[u8] = match &final_body {
                    crate::core::response::ResponseBody::Buffered(buf) => buf.as_slice(),
                    _ => &[],
                };
                crate::observe::notify_response(&crate::observe::ResponseSnapshot {
                    method: &current_method,
                    url: raw_url,
                    final_url: &final_url,
                    status,
                    request_headers: &audit_headers,
                    response_headers: &final_headers,
                    body: body_slice,
                });
            }

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

        Err(Error::Redirect(format!(
            "too many redirects (max {})",
            redirect_cap
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

/// Reorder `headers` in place so that, for each name in `order`, all
/// matching headers (case-insensitive) appear in `order`'s position.
/// Headers whose name does not appear in `order` keep their relative
/// insertion order at the tail.
///
/// Stable: multiple values for the same header keep their original
/// relative order. Unknown / dynamic names (`content-length`, `cookie`,
/// caller-anchored extras) end up at the tail untouched.
fn reorder_headers(headers: &mut Vec<(String, String)>, order: &[String]) {
    let lc_order: Vec<String> = order.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut buckets: Vec<Vec<(String, String)>> = vec![Vec::new(); lc_order.len()];
    let mut tail: Vec<(String, String)> = Vec::new();
    for h in std::mem::take(headers) {
        let lc = h.0.to_ascii_lowercase();
        match lc_order.iter().position(|n| *n == lc) {
            Some(idx) => buckets[idx].push(h),
            None => tail.push(h),
        }
    }
    for b in buckets {
        headers.extend(b);
    }
    headers.extend(tail);
}

#[cfg(test)]
mod reorder_tests {
    use super::reorder_headers;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn moves_named_headers_to_declared_order() {
        let mut headers = vec![
            h("user-agent", "u"),
            h("accept", "a"),
            h("sec-fetch-site", "s"),
            h("accept-language", "al"),
            h("sec-gpc", "1"),
        ];
        let order = vec![
            "accept".into(),
            "sec-gpc".into(),
            "accept-language".into(),
            "sec-fetch-site".into(),
        ];
        reorder_headers(&mut headers, &order);
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "accept",
                "sec-gpc",
                "accept-language",
                "sec-fetch-site",
                "user-agent",
            ]
        );
    }

    #[test]
    fn missing_names_in_order_are_skipped() {
        let mut headers = vec![h("accept", "a"), h("user-agent", "u")];
        let order = vec!["accept".into(), "sec-gpc".into(), "user-agent".into()];
        reorder_headers(&mut headers, &order);
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["accept", "user-agent"]);
    }

    #[test]
    fn case_insensitive_matching() {
        let mut headers = vec![h("User-Agent", "u"), h("Accept", "a")];
        let order = vec!["accept".into(), "user-agent".into()];
        reorder_headers(&mut headers, &order);
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["Accept", "User-Agent"]);
    }
}
