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
use crate::observe::has_observer;
use crate::util::redacted_url;

impl Session {
    /// Execute a request with an optional per-request timeout override.
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
        let timeout = override_timeout.unwrap_or(self.inner.timeouts.total);
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
            let mut cache = lock(&self.inner.url_cache);
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
        let mut current_body = body;
        let mut redirect_chain = Vec::new();
        let mut all_cookies = HashMap::new();
        let mut acc_timing = crate::core::ResponseTiming::accumulator();

        let redirect_cap = self.inner.redirect_policy.max_redirects_hint();
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

            let want_introspect = self.inner.audit_enabled || has_observer();
            let audit_headers: Vec<(String, String)> = if want_introspect {
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            } else {
                Vec::new()
            };

            let hop_body = std::mem::take(&mut current_body);
            let hop_body_was_stream = hop_body.is_stream();
            let replay_body = match &hop_body {
                Body::Bytes(b) => Some(b.clone()),
                _ => None,
            };

            let send = self.send_with_policy(
                &current_method,
                &current_url,
                headers,
                hop_body,
                stream_response,
                request_proxy,
                header_order,
            );
            let transport_resp = match self.inner.timeouts.response_header {
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
            acc_timing.add_leg(&transport_resp.timing);

            self.collect_cookies(&resp_headers, &current_url, &mut all_cookies);

            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if let Some(location) = resp_headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v.clone())
                {
                    let action = self.inner.redirect_policy.action(RedirectAttempt {
                        status,
                        url: &current_url,
                        location: Some(location.as_str()),
                        previous: &redirect_chain,
                    });
                    if action == RedirectAction::Stop {
                    } else {
                        drop(resp_body_shape);
                        redirect_chain.push(current_url.to_string());
                        current_url = current_url.join(&location)?;
                        if !matches!(current_url.scheme(), "http" | "https") {
                            return Err(Error::Redirect(format!(
                                "refusing to follow redirect to non-http(s) scheme `{}`",
                                current_url.scheme()
                            )));
                        }

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
                request_headers: audit_headers,
                tls_alpn,
                tls_peer_certificate: peer_cert_der,
                tls_version,
                tls_cipher,
                request_method: if self.inner.audit_enabled {
                    current_method.clone()
                } else {
                    String::new()
                },
                audit_tls: self
                    .inner
                    .audit_enabled
                    .then(|| std::sync::Arc::clone(&self.inner.audit_tls)),
                audit_cache: std::sync::OnceLock::new(),
                timing: acc_timing,
            });
        }

        Err(Error::Redirect(format!(
            "too many redirects (max {})",
            redirect_cap
        )))
    }

    /// Headers for one redirect-loop hop: preset or default set, brand and identity overlays, caller extras, framing length, cookies, and the identity or Gecko header order.
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
        let mut headers: Vec<HeaderPair> = if let Some(preset) = preset {
            let ctx = crate::profile::preset::HeaderContext {
                user_agent: &self.inner.user_agent,
                sec_ch_ua: &self.inner.sec_ch_ua,
                sec_ch_ua_mobile: self.inner.platform.mobile_flag(),
                sec_ch_ua_platform: self.inner.platform.sec_ch_platform(),
                accept_language: &self.inner.accept_language,
                origin,
                referer,
                firefox: self
                    .inner
                    .identity
                    .map(|id| id.http().is_firefox())
                    .unwrap_or_else(|| self.inner.browser.as_ref().is_some_and(|b| b.is_firefox())),
            };
            preset.build_headers(&ctx)
        } else {
            vec![
                (
                    "user-agent".into(),
                    Cow::Owned(self.inner.user_agent.clone()),
                ),
                ("accept".into(), Cow::Borrowed("*/*")),
                (
                    "accept-encoding".into(),
                    Cow::Borrowed("gzip, deflate, br, zstd"),
                ),
                (
                    "accept-language".into(),
                    Cow::Owned(self.inner.accept_language.clone()),
                ),
            ]
        };

        let navigate_accept_override = self
            .inner
            .identity_navigate_accept
            .as_deref()
            .or(self.inner.brand_navigate_accept.as_deref());
        if let (Some(accept_override), Some(Preset::Navigate)) = (navigate_accept_override, preset)
        {
            for (name, value) in headers.iter_mut() {
                if name == "accept" {
                    *value = Cow::Owned(accept_override.to_string());
                    break;
                }
            }
        }
        let sensitive = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower == "authorization" || lower == "proxy-authorization" || lower == "cookie"
        };

        for (k, v) in self
            .inner
            .brand_extra_headers
            .iter()
            .chain(self.inner.identity_extra_headers.iter())
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

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &sensitive);
        }

        if let Some(len) = current_body.len_hint() {
            if !matches!(current_body, Body::Empty) || len > 0 {
                headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
                headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
            }
        }

        let cross_site = crate::cookie::is_cross_site(current_url, redirect_chain);
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| current_method.eq_ignore_ascii_case(m));
        if let Some(cookie_val) =
            self.inner
                .cookie_jar
                .cookie_header_for(current_url, cross_site, safe_method)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }

        if let Some(order) = self.inner.identity_request_header_order.as_deref() {
            reorder_headers(&mut headers, order);
        } else if self.inner.browser.as_ref().is_some_and(|b| b.is_firefox()) {
            let order: Vec<String> = crate::profile::preset::FIREFOX_HEADER_ORDER
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            reorder_headers(&mut headers, &order);
        }

        headers
    }
    /// Store this hop's Set-Cookie values in the jar and accumulate the response-facing cookie view: live values read back from the jar, rejected-but-sent names reported as the server sent them.
    fn collect_cookies(
        &self,
        resp_headers: &[(crate::core::HeaderStr, crate::core::HeaderStr)],
        current_url: &url::Url,
        all_cookies: &mut HashMap<String, String>,
    ) {
        let set_cookies: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v.as_str())
            .collect();
        if !set_cookies.is_empty() {
            self.inner
                .cookie_jar
                .store_response_cookies(set_cookies.as_slice(), current_url);
            let url_str = current_url.as_str();
            for sc in &set_cookies {
                let Some((name, _)) = sc.split(';').next().and_then(|nv| nv.split_once('=')) else {
                    continue;
                };
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                if let Some(value) = self.inner.cookie_jar.get_cookie(url_str, name) {
                    all_cookies.insert(name.to_string(), value);
                } else if let Some((_, value)) = crate::cookie::rejected_cookie_name_value(sc) {
                    all_cookies.insert(name.to_string(), value);
                }
            }
        }
    }

    /// Shape the transport body for the caller: streamed as-is when streaming is on, drained then decompressed when buffering is wanted, and re-streamed as a single-chunk body when the transport buffered but the caller asked for streaming.
    async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
        stream_response: bool,
    ) -> Result<(
        crate::core::response::ResponseBody,
        Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
    )> {
        Ok(match resp_body_shape {
            crate::core::transport::TransportBody::Streaming(mut bs) if stream_response => {
                bs.set_read_timeout(self.inner.timeouts.read);
                (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Streaming(bs) => {
                let drain = drain_stream_into_vec(bs);
                let buf = if let Some(read_timeout) = self.inner.timeouts.read {
                    tokio::time::timeout(read_timeout, drain)
                        .await
                        .map_err(|_| Error::Timeout)??
                } else {
                    drain.await?
                };
                let (buf, resp_headers) =
                    decompress_and_strip(buf, resp_headers, &self.inner.compression)?;
                (
                    crate::core::response::ResponseBody::Buffered(buf),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Buffered(buf) => {
                if stream_response {
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
                        decompress_and_strip(buf, resp_headers, &self.inner.compression)?;
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

/// Referer per the browser-default `strict-origin-when-cross-origin` policy: the full previous URL on a same-origin hop, origin-only on a cross-origin hop — with credentials and fragment stripped in both cases.
fn referer_for(prev: Option<&str>, current_origin: &str) -> String {
    let Some(prev) = prev else {
        return format!("{current_origin}/");
    };
    let Ok(mut parsed) = url::Url::parse(prev) else {
        return format!("{current_origin}/");
    };
    if url_origin(&parsed) != current_origin {
        let origin = url_origin(&parsed);
        return format!("{origin}/");
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

/// Reorder `headers` in place so that, for each name in `order`, all matching headers (case-insensitive) appear in `order`'s position.
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

/// Mutex lock that survives poisoning: a panic in another request thread must not turn every later request on this session into an error.
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
