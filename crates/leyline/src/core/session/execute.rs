use http::Uri;
use url::Url;

use crate::core::headers::reorder;
use crate::core::transport::Prepared;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use crate::profile::Preset;
use crate::profile::preset::HeaderPair;

use super::Session;
use super::decompress::{decompress_and_strip, drain_stream_into_vec};
use super::header_merge::apply_extra_headers;
use crate::core::body::{Body, BodyKind};
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::{RedirectAction, RedirectAttempt};
use crate::trace;
use crate::util::redacted_url;

pub(crate) struct Attempt {
    pub(crate) method: http::Method,
    pub(crate) url: String,
    pub(crate) preset: Option<Preset>,
    pub(crate) body: Body,
    pub(crate) headers: Option<HeaderList>,
    pub(crate) timeout: Option<std::time::Duration>,
    pub(crate) timeouts: Option<crate::core::config::TimeoutConfig>,
    pub(crate) stream_response: bool,
    pub(crate) proxy: Option<String>,
    pub(crate) header_order: Option<Vec<String>>,
}

impl Attempt {
    pub(crate) fn again(
        &self,
        body: Body,
        headers: Option<HeaderList>,
        timeout: Option<std::time::Duration>,
    ) -> Attempt {
        Attempt {
            method: self.method.clone(),
            url: self.url.clone(),
            preset: self.preset,
            body,
            headers,
            timeout,
            timeouts: self.timeouts,
            stream_response: self.stream_response,
            proxy: self.proxy.clone(),
            header_order: self.header_order.clone(),
        }
    }
}

impl Session {
    pub(crate) async fn run(&self, attempt: Attempt) -> Result<Response> {
        let timeout = attempt.timeout.unwrap_or(self.inner.timeouts.total);
        let inner: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(attempt));
        trace::scope(self.inner.trace.as_ref(), async move {
            let out = match tokio::time::timeout(timeout, inner).await {
                Ok(result) => result,
                Err(_) => Err(Error::new(Kind::Timeout)),
            };
            trace::done(match &out {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            });
            out
        })
        .await
    }

    #[tracing::instrument(
        name = "session.execute",
        level = "debug",
        skip_all,
        fields(http.method = attempt.method.as_str(), http.url = redacted_url(&attempt.url))
    )]
    async fn execute_inner(&self, attempt: Attempt) -> Result<Response> {
        let Attempt {
            method,
            url: raw_url,
            preset,
            body,
            headers: extra_headers,
            timeouts: over,
            stream_response,
            proxy: request_proxy,
            header_order,
            ..
        } = attempt;
        let read = over.and_then(|t| t.read).or(self.inner.timeouts.read);
        let raw_url = raw_url.as_str();
        let request_proxy = request_proxy.as_deref();
        let header_order = header_order.as_deref();
        let mut current_url = {
            let mut cache = lock(&self.inner.url_cache);
            match cache.as_mut() {
                Some((raw, parsed)) if raw == raw_url => Arc::clone(parsed),
                _ => {
                    let parsed =
                        Arc::new(Url::parse(raw_url).map_err(crate::core::Error::from_url_parse)?);
                    *cache = Some((raw_url.to_string(), Arc::clone(&parsed)));
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
            let origin = if redirect_chain.is_empty() {
                Cow::Borrowed(original_origin.as_str())
            } else {
                Cow::Owned(url_origin(&current_url))
            };
            let referer = referer_for(redirect_chain.last().map(|s: &String| s.as_str()), &origin);

            let strip_sensitive = !redirect_chain.is_empty() && origin.as_ref() != original_origin;
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
                header_order,
            );

            let want_introspect = self.inner.audit_tls.is_some();
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
            let replay_body = hop_body.as_bytes().cloned();

            let send = self.dispatch(Prepared {
                method: &current_method,
                url: &current_url,
                headers,
                body: hop_body,
                proxy: request_proxy,
                stream_response,
            });
            let ttfb = over
                .and_then(|t| t.response_header)
                .or(self.inner.timeouts.response_header);
            let transport_resp = match ttfb {
                Some(ttfb) => tokio::time::timeout(ttfb, send)
                    .await
                    .map_err(|_| Error::new(Kind::Timeout))??,
                None => send.await?,
            };
            let status = transport_resp.status;
            let resp_headers = transport_resp.headers;
            let resp_trailers = transport_resp.trailers;
            let resp_body_shape = transport_resp.body;
            let final_url = transport_resp.final_url;
            let response_version = transport_resp.version;
            let tls_alpn = transport_resp.tls_alpn;
            let peer_cert_der = transport_resp.peer_cert_der;
            let tls_version = transport_resp.tls_version;
            let tls_cipher = transport_resp.tls_cipher;
            acc_timing.add_leg(&transport_resp.timing);

            self.collect_cookies(&resp_headers, &current_url, &mut all_cookies);
            #[cfg(feature = "http3")]
            if let Some(host) = current_url.host_str()
                && let Some(port) = current_url.port_or_known_default()
            {
                for (_, v) in resp_headers.iter().filter(|(k, _)| *k == "alt-svc") {
                    self.inner.pool.note_alt_svc(
                        host,
                        port,
                        &String::from_utf8_lossy(v.as_bytes()),
                    );
                }
            }

            let code = status.as_u16();
            if matches!(code, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = resp_headers
                    .iter()
                    .find(|(k, _)| *k == "location")
                    .map(|(_, v)| String::from_utf8_lossy(v.as_bytes()).into_owned())
            {
                let attempt_url: Uri = current_url.as_str().parse().unwrap_or_default();
                let action = self.inner.redirect_policy.action(RedirectAttempt {
                    status: code,
                    url: &attempt_url,
                    location: Some(location.as_str()),
                    previous: &redirect_chain,
                });
                if action == RedirectAction::Stop {
                } else {
                    drop(resp_body_shape);
                    redirect_chain.push(current_url.to_string());
                    current_url = Arc::new(
                        current_url
                            .join(&location)
                            .map_err(crate::core::Error::from_url_parse)?,
                    );
                    if !matches!(current_url.scheme(), "http" | "https") {
                        return Err(Error::new(Kind::Redirect).with_message(format!(
                            "refusing to follow redirect to non-http(s) scheme `{}`",
                            current_url.scheme()
                        )));
                    }

                    if matches!(code, 301..=303) {
                        current_method = "GET".to_string();
                        current_body = Body::default();
                    } else if hop_body_was_stream {
                        return Err(Error::new(Kind::Redirect).with_message(format!(
                            "cannot follow {code} redirect: streaming request bodies are \
                             not replayable. Either buffer the body before sending or set \
                             max_redirects(0)."
                        )));
                    } else if let Some(bytes) = replay_body {
                        current_body = Body::bytes(bytes);
                    }
                    continue;
                }
            }

            let (final_body, final_headers) = self
                .finalize_response_body(resp_body_shape, resp_headers, stream_response, read)
                .await?;

            return Ok(Response {
                status,
                headers: final_headers,
                body: final_body,
                cookies: all_cookies,
                url: final_url,
                redirect_chain,
                version: response_version,
                trailers: resp_trailers,
                request_headers: audit_headers,
                tls_alpn,
                tls_peer_certificate: peer_cert_der,
                tls_version,
                tls_cipher,
                request_method: if self.inner.audit_tls.is_some() {
                    current_method.clone()
                } else {
                    String::new()
                },
                audit_tls: self.inner.audit_tls.clone(),
                audit_cache: std::sync::OnceLock::new(),
                compression: self.inner.compression,
                timing: acc_timing,
            });
        }

        Err(Error::new(Kind::Redirect)
            .with_message(format!("too many redirects (max {})", redirect_cap)))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_hop_headers(
        &self,
        preset: Option<Preset>,
        origin: &str,
        referer: &str,
        current_url: &Url,
        current_method: &str,
        redirect_chain: &[String],
        current_body: &Body,
        extra_headers: Option<&HeaderList>,
        strip_sensitive: bool,
        header_order: Option<&[String]>,
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
                .map(|h| h.iter().any(|(uk, _)| uk.as_str().eq_ignore_ascii_case(k)))
                .unwrap_or(false);
            if user_has_it || (strip_sensitive && sensitive(k)) {
                continue;
            }
            match headers
                .iter()
                .position(|(hk, _)| hk.eq_ignore_ascii_case(k))
            {
                Some(pos) => headers[pos].1 = Cow::Owned(v.clone()),
                None => headers.push((Cow::Owned(k.clone()), Cow::Owned(v.clone()))),
            }
        }

        if let Some(extra) = extra_headers {
            apply_extra_headers(&mut headers, extra, strip_sensitive, &sensitive);
        }

        if let Some(len) = current_body.len_hint()
            && (!matches!(current_body.0, BodyKind::Empty) || len > 0)
        {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-length"));
            headers.insert(0, ("content-length".into(), Cow::Owned(len.to_string())));
        }

        let cross_site = crate::cookie::is_cross_site(current_url, redirect_chain);
        let safe_method = ["GET", "HEAD"]
            .iter()
            .any(|m| current_method.eq_ignore_ascii_case(m));
        let caller_cookie = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("cookie"));
        if !caller_cookie
            && let Some(cookie_val) =
                self.inner
                    .cookie_jar
                    .cookie_header_for(current_url, cross_site, safe_method)
        {
            headers.push(("cookie".into(), Cow::Owned(cookie_val)));
        }

        if let Some(order) = header_order {
            reorder(&mut headers, order);
        } else if let Some(order) = self.inner.identity_request_header_order.as_deref() {
            reorder(&mut headers, order);
        } else if self.inner.browser.as_ref().is_some_and(|b| b.is_firefox()) {
            let order: Vec<String> = crate::profile::preset::FIREFOX_HEADER_ORDER
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            reorder(&mut headers, &order);
        }

        headers
    }
    fn collect_cookies(
        &self,
        resp_headers: &[(http::HeaderName, http::HeaderValue)],
        current_url: &Url,
        all_cookies: &mut HashMap<String, String>,
    ) {
        let set_cookies: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| *k == "set-cookie")
            .filter_map(|(_, v)| v.to_str().ok())
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

    async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: Vec<(http::HeaderName, http::HeaderValue)>,
        stream_response: bool,
        read: Option<std::time::Duration>,
    ) -> Result<(
        crate::core::response::ResponseBody,
        Vec<(http::HeaderName, http::HeaderValue)>,
    )> {
        Ok(match resp_body_shape {
            crate::core::transport::TransportBody::Streaming(mut bs) if stream_response => {
                bs.set_read_timeout(read);
                (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Streaming(bs) => {
                let drain = drain_stream_into_vec(bs);
                let buf = if let Some(read_timeout) = read {
                    tokio::time::timeout(read_timeout, drain)
                        .await
                        .map_err(|_| Error::new(Kind::Timeout))??
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

fn url_origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

fn referer_for(prev: Option<&str>, current_origin: &str) -> String {
    let Some(prev) = prev else {
        return format!("{current_origin}/");
    };
    let Ok(mut parsed) = Url::parse(prev) else {
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

#[cfg(test)]
mod redact_tests;
#[cfg(test)]
mod referer_tests;
#[cfg(test)]
mod reorder_tests;

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
