use http::Uri;
use url::Url;

use crate::core::transport::Prepared;
use std::borrow::Cow;
use std::sync::Arc;

use crate::profile::Preset;

use super::Session;
use crate::core::body::Body;
use crate::core::config::TimeoutConfig;
use crate::core::deadline::Deadline;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::Response;
use crate::core::{RedirectAction, RedirectAttempt, RedirectPolicy};
use crate::trace;
use crate::util::redact;

mod headers;
mod response;

pub(crate) struct Attempt {
    pub(crate) method: http::Method,
    pub(crate) url: String,
    pub(crate) preset: Option<Preset>,
    pub(crate) body: Body,
    pub(crate) headers: Option<HeaderList>,
    pub(crate) deadline: Deadline,
    pub(crate) stream_response: bool,
    pub(crate) proxy: Option<String>,
    pub(crate) header_order: Option<Vec<String>>,
    pub(crate) redirect: Option<RedirectPolicy>,
}

impl Attempt {
    pub(crate) fn again(&self, body: Body, headers: Option<HeaderList>) -> Attempt {
        Attempt {
            method: self.method.clone(),
            url: self.url.clone(),
            preset: self.preset,
            body,
            headers,
            deadline: self.deadline,
            stream_response: self.stream_response,
            proxy: self.proxy.clone(),
            header_order: self.header_order.clone(),
            redirect: self.redirect.clone(),
        }
    }
}

impl Session {
    pub(crate) fn deadline(
        &self,
        request: Option<&TimeoutConfig>,
        total: Option<std::time::Duration>,
    ) -> Deadline {
        Deadline::new(&self.inner.timeouts, request, total)
    }

    pub(crate) async fn attempt(&self, attempt: Attempt) -> Result<Response> {
        let deadline = attempt.deadline;
        let inner: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(attempt));
        trace::scope(self.inner.trace.as_ref(), async move {
            let out = deadline.total(inner).await;
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
        fields(http.method = attempt.method.as_str(), http.url = redact(&attempt.url))
    )]
    async fn execute_inner(&self, attempt: Attempt) -> Result<Response> {
        let Attempt {
            method,
            url: raw_url,
            preset,
            body,
            headers: extra_headers,
            deadline,
            stream_response,
            proxy: request_proxy,
            header_order,
            redirect,
        } = attempt;
        let redirect_policy = redirect.as_ref().unwrap_or(&self.inner.redirect_policy);
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
        let mut acc_timing = crate::core::ResponseTiming::accumulator();

        let redirect_cap = redirect_policy.max_redirects_hint();
        for _ in 0..=redirect_cap {
            let origin = if redirect_chain.is_empty() {
                Cow::Borrowed(original_origin.as_str())
            } else {
                Cow::Owned(url_origin(&current_url))
            };
            let referer = referer_for(redirect_chain.last().map(|s: &String| s.as_str()), &origin);

            let strip_sensitive = !redirect_chain.is_empty() && origin.as_ref() != original_origin;
            let headers = self.attempt_headers(
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

            let step_body = std::mem::take(&mut current_body);
            let replay_body = step_body.replay();

            let send = self.send_with_policy(Prepared {
                method: &current_method,
                url: &current_url,
                headers,
                body: step_body,
                proxy: request_proxy,
                stream_response,
            });
            let transport_resp = deadline.response_header(send).await?;
            let status = transport_resp.status;
            let resp_headers = transport_resp.headers;
            let resp_trailers = transport_resp.trailers;
            let resp_body_shape = transport_resp.body;
            let final_url = transport_resp.final_url;
            let response_version = transport_resp.version;
            let tls = transport_resp.tls;
            acc_timing.add_leg(&transport_resp.timing);

            self.store_cookies(&resp_headers, &current_url);
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
                let action = redirect_policy.action(RedirectAttempt {
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
                    } else if let Some(replay) = replay_body {
                        current_body = replay;
                    } else {
                        return Err(Error::new(Kind::Redirect).with_message(format!(
                            "cannot follow {code} redirect: streaming request bodies are \
                             not replayable. Either buffer the body before sending or set \
                             max_redirects(0)."
                        )));
                    }
                    continue;
                }
            }

            let (final_body, final_headers) = self
                .finalize_response_body(resp_body_shape, resp_headers, stream_response, &deadline)
                .await?;

            return Ok(Response {
                status,
                headers: final_headers,
                body: final_body,
                url: final_url,
                redirect_chain,
                version: response_version,
                trailers: resp_trailers,
                request_headers: audit_headers,
                tls,
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
