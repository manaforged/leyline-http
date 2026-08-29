use std::borrow::Cow;
use std::collections::HashMap;

use crate::profile::Preset;
use crate::profile::preset::HeaderPair;

use super::Session;
use super::decompress::{decompress_and_strip, drain_stream_into_vec};
use super::header_merge::apply_extra_headers;
use crate::core::body::Body;
use crate::core::error::{Error, Result};
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::{RedirectAction, RedirectAttempt};
use crate::util::redacted_url;

impl Session {
    // Core execution.

    /// Execute a request with an optional per-request timeout override.
    /// When `override_timeout` is `None`, the session-level timeout applies.
    /// `request_proxy`, when `Some`, overrides the session's default proxy
    /// for this single request (and any redirects it follows).
    /// Handles redirects, decompression, cookies.
    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
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
        header_order: Option<&[String]>,
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
            header_order,
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
        fields(http.method = method, http.url = redacted_url(raw_url))
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    async fn execute_inner(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        stream_response: bool,
        request_proxy: Option<&str>,
        header_order: Option<&[String]>,
    ) -> Result<Response> {
        let mut current_url = {
            // Sequential calls with the same URL string are the dominant
            // shape; the cache trades a parse for a string compare.
            let mut cache = lock(&self.url_cache);
            match cache.as_mut() {
                Some((raw, parsed)) if raw == raw_url => parsed.clone(),
                _ => {
                    let parsed = url::Url::parse(raw_url)?;
                    *cache = Some((raw_url.to_string(), parsed.clone()));
                    parsed
                }
            }
        };
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
        // Accumulates timing across every redirect leg so the returned
        // `Response::timing()` describes the whole call, not just the last hop.
        let mut acc_timing = crate::core::ResponseTiming::accumulator();

        let redirect_cap = self.redirect_policy.max_redirects_hint();
        for _ in 0..=redirect_cap {
            let origin = url_origin(&current_url);
            let referer = referer_for(redirect_chain.last().map(|s: &String| s.as_str()), &origin);

            let strip_sensitive = !redirect_chain.is_empty() && origin != original_origin;
            let headers = self.build_hop_headers(
                preset,
                &origin,
                &referer,
                &current_url,
                &current_method,
                &redirect_chain,
                &current_body,
                extra_headers.as_ref(),
                strip_sensitive,
            );

            // Clone request headers only when audit is on.
            let want_introspect = self.audit_enabled;
            let audit_headers: Vec<(String, String)> = if want_introspect {
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            } else {
                Vec::new()
            };

            // Take the body for this hop. Streams are one-shot; we replace
            // `current_body` with `Body::Empty` so a follow-up redirect
            // sees there's nothing to replay (and fails loudly). A buffered
            // body is kept as a cheap refcounted `Bytes` clone in
            // `replay_body` so a 307/308 (method+body-preserving) redirect can
            // re-send it — without this the hop body is moved into the send and
            // the redirected request would go out with an empty body.
            let hop_body = std::mem::take(&mut current_body);
            let hop_body_was_stream = hop_body.is_stream();
            let replay_body = match &hop_body {
                Body::Bytes(b) => Some(b.clone()),
                _ => None,
            };

            // Send via the configured protocol policy. The per-request
            // proxy override (if any) carries through every redirect
            // hop in this `execute_inner` call — once a caller picks a
            // proxy for a request, all redirects of that request go
            // through the same proxy.
            //
            // `send_with_policy` resolves when the transport response is ready:
            // at headers for a streamed response (true TTFB), but only after the
            // full body for the default buffered response (every protocol). So
            // `response_header` caps first-byte for streamed callers and the
            // whole response per hop for buffered callers — either way a proxy
            // that connects then goes silent errors here instead of hanging out
            // to `total`. See `TimeoutConfig::response_header`.
            let send = self.send_with_policy(
                &current_method,
                &current_url,
                headers,
                hop_body,
                stream_response,
                request_proxy,
                header_order,
            );
            let transport_resp = match self.timeouts.response_header {
                Some(ttfb) => tokio::time::timeout(ttfb, send)
                    .await
                    .map_err(|_| Error::Timeout)??,
                None => send.await?,
            };
            let status = transport_resp.status;
            let resp_headers = transport_resp.headers;
            let resp_body_shape = transport_resp.body;
            let final_url = transport_resp.final_url;
            let response_version = transport_resp.version;
            let tls_alpn = transport_resp.tls_alpn;
            let peer_cert_der = transport_resp.peer_cert_der;
            let tls_version = transport_resp.tls_version;
            let tls_cipher = transport_resp.tls_cipher;
            // Fold this leg's timing into the running total. On a redirect the
            // loop continues and the next leg adds to it; the non-redirect
            // return below ships the accumulated whole-request breakdown.
            acc_timing.add_leg(&transport_resp.timing);

            self.collect_cookies(&resp_headers, &current_url, &mut all_cookies);

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
                        // Refuse to follow a redirect to a non-HTTP(S) target
                        // (`file:`, `data:`, `javascript:`, …). Browsers reject
                        // these outright; without this guard the target would
                        // flow into the transport and fail later with a
                        // confusing "requires an https:// URL" error.
                        if !matches!(current_url.scheme(), "http" | "https") {
                            return Err(Error::Redirect(format!(
                                "refusing to follow redirect to non-http(s) scheme `{}`",
                                current_url.scheme()
                            )));
                        }

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
                        } else if let Some(bytes) = replay_body {
                            // 307/308 preserve method and body; re-send the
                            // buffered payload on the next hop (an empty body
                            // stays empty — `replay_body` is None).
                            current_body = Body::Bytes(bytes);
                        }
                        continue;
                    }
                }
            }

            let (final_body, final_headers) = self
                .finalize_response_body(resp_body_shape, resp_headers, stream_response)
                .await?;

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
                    response_headers_raw: &final_headers,
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
                // Move (not clone) the retained headers into the response —
                // empty when introspection is off.
                request_headers: audit_headers,
                tls_alpn,
                tls_peer_certificate: peer_cert_der,
                tls_version,
                tls_cipher,
                // Audit is opt-in: when off, `audit()` returns None and we
                // store neither the cache handle nor the request method. When
                // on, the Arc clone is one refcount bump and JA4H is still
                // deferred to the first `audit()` call.
                request_method: if self.audit_enabled {
                    current_method.clone()
                } else {
                    String::new()
                },
                audit_tls: self
                    .audit_enabled
                    .then(|| std::sync::Arc::clone(&self.audit_tls)),
                audit_cache: std::sync::OnceLock::new(),
                timing: acc_timing,
            });
        }

        Err(Error::Redirect(format!(
            "too many redirects (max {})",
            redirect_cap
        )))
    }

    /// Headers for one redirect-loop hop: preset or default set, brand and
    /// identity overlays, caller extras, framing length, cookies, and the
    /// identity or Gecko header order.
    #[allow(clippy::too_many_arguments)]
    fn build_hop_headers(
        &self,
        preset: Option<Preset>,
        origin: &str,
        referer: &str,
        current_url: &url::Url,
        current_method: &str,
        redirect_chain: &[String],
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
    ) -> Vec<HeaderPair> {
        // Build headers.
        let mut headers: Vec<HeaderPair> = if let Some(preset) = preset {
            let ctx = crate::profile::preset::HeaderContext {
                user_agent: &self.user_agent,
                sec_ch_ua: &self.sec_ch_ua,
                sec_ch_ua_mobile: self.platform.mobile_flag(),
                sec_ch_ua_platform: self.platform.sec_ch_platform(),
                accept_language: &self.accept_language,
                origin,
                referer,
                firefox: self
                    .identity
                    .map(|id| id.http().is_firefox())
                    .unwrap_or_else(|| self.browser.as_ref().is_some_and(|b| b.is_firefox())),
            };
            preset.build_headers(&ctx)
        } else {
            vec![
                ("user-agent".into(), Cow::Owned(self.user_agent.clone())),
                ("accept".into(), Cow::Borrowed("*/*")),
                (
                    "accept-encoding".into(),
                    Cow::Borrowed("gzip, deflate, br, zstd"),
                ),
                (
                    "accept-language".into(),
                    Cow::Owned(self.accept_language.clone()),
                ),
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
        if let (Some(accept_override), Some(Preset::Navigate)) = (navigate_accept_override, preset)
        {
            for (name, value) in headers.iter_mut() {
                if name == "accept" {
                    *value = Cow::Owned(accept_override.to_string());
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
        // Cross-origin redirect hops drop credential-bearing headers —
        // the rule covers session-level extras (brand overlays,
        // identity extras) exactly like per-request ones.
        let sensitive = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower == "authorization" || lower == "proxy-authorization" || lower == "cookie"
        };

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
            if !user_has_it && !preset_has_it && !(strip_sensitive && sensitive(k)) {
                headers.push((Cow::Owned(k.clone()), Cow::Owned(v.clone())));
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
        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &sensitive);
        }

        // Content-Length for requests with a known-length body.
        // For length-unknown streams we leave it out and let the
        // transport pick `Transfer-Encoding: chunked` (H1) or native
        // framing (H2/H3).
        if let Some(len) = current_body.len_hint() {
            if !matches!(current_body, Body::Empty) || len > 0 {
                // The computed length is authoritative for a known-length
                // body. Drop any caller-supplied content-length so we never
                // emit two (a request-smuggling shape) or a stale value.
                headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
                // Chrome emits `content-length` as the FIRST regular
                // header (right after the pseudo headers on H2, right
                // after Host/Connection on H1) — not at the tail. The
                // position is part of the wire fingerprint Akamai-class
                // edges score on POSTs.
                headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
            }
        }

        // Cookies. SameSite is enforced against the request's cross-site
        // context: a cross-site redirect withholds `Strict` cookies (and
        // `Lax` on non-safe methods), matching a real browser navigation.
        let cross_site = crate::cookie::is_cross_site(current_url, redirect_chain);
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| current_method.eq_ignore_ascii_case(m));
        if let Some(cookie_val) =
            self.cookie_jar
                .cookie_header_for(current_url, cross_site, safe_method)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }

        // Identity-level header reordering. Browsers like Brave ship
        // a non-Chrome request-header sequence (e.g. accept-language
        // repositioned after sec-gpc, between accept and sec-fetch-*).
        // When the active identity declares a `request_header_order`,
        // sort the assembled headers to that order; names not in the
        // list keep their relative position at the tail.
        if let Some(order) = self.identity_request_header_order.as_deref() {
            reorder_headers(&mut headers, order);
        } else if self.browser.as_ref().is_some_and(|b| b.is_firefox()) {
            // Firefox ships no per-identity TOML order; apply the built-in Gecko header order so
            // the full request sequence (including the just-assembled `cookie`) matches real
            // Firefox rather than the Chrome-shaped preset.
            let order: Vec<String> = crate::profile::preset::FIREFOX_HEADER_ORDER
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            reorder_headers(&mut headers, &order);
        }

        headers
    }
    /// Store this hop's Set-Cookie values in the jar and accumulate the
    /// response-facing cookie view: live values read back from the jar,
    /// rejected-but-sent names reported as the server sent them.
    fn collect_cookies(
        &self,
        resp_headers: &[(crate::core::HeaderStr, crate::core::HeaderStr)],
        current_url: &url::Url,
        all_cookies: &mut HashMap<String, String>,
    ) {
        // Store cookies from response and accumulate across redirect chain.
        let set_cookies: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v.as_str())
            .collect();
        if !set_cookies.is_empty() {
            self.cookie_jar
                .store_response_cookies(set_cookies.as_slice(), current_url);
            // Response-facing cookie map: read each value back from the jar,
            // which parsed it with the one RFC 6265 parser (quote-stripping,
            // domain/prefix validation). We take the name from the header
            // (split on the first `;` then the first `=`, exactly as the
            // parser does) but the *value* from the jar, so
            // `Response::cookies()` cannot diverge from the jar on quoted
            // values or attribute edge cases. A header the jar
            // rejected (bad domain, public suffix, `__Host-`/`__Secure-`
            // violation) or a deletion (`Max-Age=0`) is not a live cookie
            // and is correctly absent. `get_cookie` scopes to this URL, so a
            // cookie the server pinned to a non-matching path is reported by
            // the jar rather than echoed raw here.
            let url_str = current_url.as_str();
            for sc in &set_cookies {
                let Some((name, _)) = sc.split(';').next().and_then(|nv| nv.split_once('=')) else {
                    continue;
                };
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                if let Some(value) = self.cookie_jar.get_cookie(url_str, name) {
                    all_cookies.insert(name.to_string(), value);
                } else if let Some((_, value)) = crate::cookie::rejected_cookie_name_value(sc) {
                    // The jar rejected this header for storage (bad domain,
                    // public suffix, `__Host-`/`__Secure-` violation), so it
                    // can never broadcast on later requests — but the server
                    // did send it, and the response view reports what the
                    // server sent (reqwest parity). Deletions and malformed
                    // headers stay hidden.
                    all_cookies.insert(name.to_string(), value);
                }
            }
        }
    }

    /// Shape the transport body for the caller: streamed as-is when
    /// streaming is on, drained then decompressed when buffering is
    /// wanted, and re-streamed as a single-chunk body when the transport
    /// buffered but the caller asked for streaming.
    async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
        stream_response: bool,
    ) -> Result<(
        crate::core::response::ResponseBody,
        Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
    )> {
        // If the caller opted into streaming, deliver as-is WITHOUT
        // decompression. Otherwise materialise and decompress as today.
        Ok(match resp_body_shape {
            crate::core::transport::TransportBody::Streaming(mut bs) if stream_response => {
                // The session read_timeout becomes a per-chunk idle timeout
                // on the delivered stream.
                bs.set_read_timeout(self.timeouts.read);
                (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                )
            }
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
                let (buf, resp_headers) =
                    decompress_and_strip(buf, resp_headers, &self.compression)?;
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
                            crate::core::body_stream::BodyStream::from_bytes(bytes::Bytes::from(
                                buf,
                            )),
                        ),
                        resp_headers,
                    )
                } else {
                    let (buf, resp_headers) =
                        decompress_and_strip(buf, resp_headers, &self.compression)?;
                    (
                        crate::core::response::ResponseBody::Buffered(buf),
                        resp_headers,
                    )
                }
            }
        })
    }
}

fn url_origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

/// Referer per the browser-default `strict-origin-when-cross-origin`
/// policy: the full previous URL on a same-origin hop, origin-only on a
/// cross-origin hop — with credentials and fragment stripped in both cases.
fn referer_for(prev: Option<&str>, current_origin: &str) -> String {
    let Some(prev) = prev else {
        return format!("{current_origin}/");
    };
    let Ok(mut parsed) = url::Url::parse(prev) else {
        return format!("{current_origin}/");
    };
    // Cross-origin hop: the referrer is the SOURCE origin (where the
    // redirect came from), never the destination.
    if url_origin(&parsed) != current_origin {
        let origin = url_origin(&parsed);
        return format!("{origin}/");
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

/// Reorder `headers` in place so that, for each name in `order`, all
/// matching headers (case-insensitive) appear in `order`'s position.
/// Headers whose name does not appear in `order` keep their relative
/// insertion order at the tail.
///
/// Stable: multiple values for the same header keep their original
/// relative order. Unknown / dynamic names (`content-length`, `cookie`,
/// caller-anchored extras) end up at the tail untouched.
pub(crate) fn reorder_headers(headers: &mut Vec<HeaderPair>, order: &[String]) {
    let lc_order: Vec<String> = order.iter().map(|s| s.to_ascii_lowercase()).collect();
    let mut buckets: Vec<Vec<HeaderPair>> = vec![Vec::new(); lc_order.len()];
    let mut tail: Vec<HeaderPair> = Vec::new();
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
mod redact_tests;
#[cfg(test)]
mod referer_tests;
#[cfg(test)]
mod reorder_tests;

/// Mutex lock that survives poisoning: a panic in another request thread must
/// not turn every later request on this session into an error.
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
