use url::Url;

use crate::FetchSite;
use crate::core::transport::{Prepared, ResponseMode};
use std::sync::Arc;

use crate::profile::Preset;

use self::auth::{DigestLeg, unreplayable_body};
use self::journey::{Journey, redirect_location};
use super::Session;
use crate::core::body::Body;
use crate::core::config::TimeoutConfig;
use crate::core::deadline::Deadline;
use crate::core::digest::DigestAuth;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::{Response, ResponseBody};
use crate::core::{ProxyConfig, RedirectPolicy};
use crate::trace;
use crate::util::{lock, redact, without_userinfo};

mod auth;
mod headers;
mod journey;
mod response;

pub(crate) struct Attempt {
    pub(crate) method: http::Method,
    pub(crate) url: String,
    pub(crate) preset: Option<Preset>,
    pub(crate) body: Body,
    pub(crate) headers: Option<HeaderList>,
    pub(crate) deadline: Deadline,
    pub(crate) response: ResponseMode,
    pub(crate) proxy: Option<ProxyConfig>,
    pub(crate) header_order: Option<Vec<String>>,
    pub(crate) redirect: Option<RedirectPolicy>,
    pub(crate) digest: Option<DigestAuth>,
    pub(crate) initiator: Option<Url>,
    pub(crate) trusted_origin: Option<Url>,
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
            response: self.response,
            proxy: self.proxy.clone(),
            header_order: self.header_order.clone(),
            redirect: self.redirect.clone(),
            digest: self.digest.clone(),
            initiator: self.initiator.clone(),
            trusted_origin: self.trusted_origin.clone(),
        }
    }
}

impl Session {
    pub(crate) fn deadline(&self, request: Option<&TimeoutConfig>) -> Deadline {
        Deadline::new(&self.inner.timeouts, request)
    }

    pub(crate) async fn attempt(&self, attempt: Attempt) -> Result<Response> {
        let deadline = attempt.deadline;
        let inner: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(attempt));
        trace::scope(self.inner.trace.as_ref(), async move {
            let out = self.unless_shut_down(deadline.total(inner)).await;
            trace::done(match &out {
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            });
            out
        })
        .await
    }

    pub(crate) async fn unless_shut_down<T>(
        &self,
        fut: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::select! {
            biased;
            () = self.inner.shutdown.cancelled() => Err(Error::shut_down()),
            out = fut => out,
        }
    }

    pub fn shutdown(&self) {
        self.inner.shutdown.cancel();
    }

    pub fn is_shut_down(&self) -> bool {
        self.inner.shutdown.is_cancelled()
    }

    pub(crate) async fn traced<F>(
        &self,
        method: &str,
        original_url: &str,
        streamed: bool,
        request: F,
    ) -> Result<Response>
    where
        F: std::future::Future<Output = (Result<Response>, u32)>,
    {
        let hook = self.inner.trace.as_ref();
        trace::scope(hook, async move {
            let (out, attempts) = request.await;
            if hook.is_some() {
                let finish = trace::Finish {
                    method,
                    original_url,
                    attempts,
                    streamed,
                };
                trace::summary(&finish, out.as_ref());
            }
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
            response,
            proxy: request_proxy,
            header_order,
            redirect,
            digest,
            initiator,
            trusted_origin,
        } = attempt;
        let redirect_policy = redirect.as_ref().unwrap_or(&self.inner.redirect_policy);
        let request_proxy = request_proxy.as_ref();
        let header_order = header_order.as_deref();
        let mut journey = Journey::begin(
            self.hsts_upgrade(&self.resolve_url(&raw_url)?),
            method.to_string(),
            body,
            extra_headers,
            initiator,
            self.session_referer(),
        )
        .trusting(trusted_origin.as_ref());

        let mut digest = digest.map(DigestLeg::new);
        let redirect_cap = redirect_policy.max_redirects_hint();
        let mut pass = None;
        let mut answered = false;
        loop {
            journey.url = self.hsts_upgrade(&journey.url);
            drop(pass.take());
            pass = self.inner.host_limits.admit(&journey.url).await;
            let headers = self.leg_headers(&journey, preset, header_order);
            journey.authorized = None;
            let audit_headers = self.audit_copy(&headers);

            let step_body = std::mem::take(&mut journey.body);
            let replay_body = step_body.replay();

            let proxy = self
                .proxy_for(&journey.url, request_proxy)
                .map_err(leg_error(answered))?;
            let send = self.send_with_policy(Prepared {
                method: &journey.method,
                url: &journey.url,
                headers,
                body: step_body,
                proxy,
                response,
            });
            let leg = deadline
                .response_header(send)
                .await
                .map_err(leg_error(answered))?;
            answered |= leg.status != http::StatusCode::UNAUTHORIZED;
            journey.timing.add_leg(&leg.timing);

            self.store_cookies(&leg.headers, &journey.url);
            self.note_hsts(&journey.url, &leg.headers);
            #[cfg(feature = "http3")]
            self.note_alt_svc(&journey.url, &leg.headers);

            let next = Turn {
                code: leg.status.as_u16(),
                headers: &leg.headers,
                redirect_policy,
                redirect_cap,
            };
            if next.take(&mut journey, digest.as_mut(), replay_body)? {
                drop(leg.body);
                continue;
            }

            let mut response = self
                .assemble_response(leg, journey, audit_headers, response, &deadline)
                .await?;
            if let ResponseBody::Streaming(body) = &mut response.body {
                body.hold(pass.take());
            }
            return Ok(response);
        }
    }

    fn session_referer(&self) -> Option<&str> {
        self.inner
            .default_headers
            .iter()
            .rfind(|(k, _)| k.eq_ignore_ascii_case("referer"))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    fn resolve_url(&self, raw_url: &str) -> Result<Arc<Url>> {
        let mut cache = lock(&self.inner.url_cache);
        match cache.as_mut() {
            Some((raw, parsed)) if raw == raw_url => Ok(Arc::clone(parsed)),
            _ => {
                let parsed = Arc::new(Url::parse(raw_url).map_err(Error::from_url_parse)?);
                *cache = Some((raw_url.to_string(), Arc::clone(&parsed)));
                Ok(parsed)
            }
        }
    }
}

pub(super) struct RequestContext<'a> {
    pub(super) origin: &'a str,
    pub(super) origin_downgrade: bool,
    pub(super) referer: &'a str,
    pub(super) initiated: bool,
    pub(super) site: SiteContext<'a>,
}

pub(in crate::core::session) struct SiteContext<'a> {
    pub(in crate::core::session) fetch_site: FetchSite,
    pub(in crate::core::session) initiator: Option<&'a Url>,
    pub(in crate::core::session) chain: &'a [Url],
}

fn fetch_site_for(initiator: Option<&Url>, chain: &[Url], current: &Url) -> FetchSite {
    let Some(initiator) = initiator else {
        return FetchSite::CrossSite;
    };
    FetchSite::across(initiator, chain.iter().chain(std::iter::once(current)))
}

fn leg_error(answered: bool) -> impl Fn(Error) -> Error {
    move |err| if answered { err.after_response() } else { err }
}

pub(crate) fn url_origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

pub(crate) fn referer_for(prev: Option<&str>, current_origin: &str) -> String {
    let Some(prev) = prev else {
        return format!("{current_origin}/");
    };
    let Ok(parsed) = Url::parse(prev) else {
        return format!("{current_origin}/");
    };
    if parsed.scheme() == "https" && !current_origin.starts_with("https:") {
        return String::new();
    }
    if url_origin(&parsed) != current_origin {
        let origin = url_origin(&parsed);
        return format!("{origin}/");
    }
    let mut parsed = without_userinfo(parsed);
    parsed.set_fragment(None);
    parsed.to_string()
}

#[cfg(all(test, feature = "http3"))]
mod alt_svc_tests;
#[cfg(test)]
mod redact_tests;
#[cfg(test)]
mod referer_tests;
#[cfg(test)]
mod reorder_tests;

struct Turn<'a> {
    code: u16,
    headers: &'a [(http::HeaderName, http::HeaderValue)],
    redirect_policy: &'a RedirectPolicy,
    redirect_cap: usize,
}

impl Turn<'_> {
    fn take(
        &self,
        journey: &mut Journey,
        digest: Option<&mut DigestLeg>,
        replay_body: Option<Body>,
    ) -> Result<bool> {
        if let Some(location) = redirect_location(self.code, self.headers)
            && let Some(hop) = journey.approved_hop(self.redirect_policy, self.code, &location)
        {
            journey.follow(hop, self.code, &location, replay_body)?;
            if journey.chain.len() > self.redirect_cap {
                return Err(Error::new(Kind::Redirect)
                    .with_message(format!("too many redirects (max {})", self.redirect_cap)));
            }
            if let Some(digest) = digest {
                journey.authorized = digest.next_hop(journey)?;
            }
            return Ok(true);
        }
        if self.code == 401
            && let Some(digest) = digest
            && let Some(authorized) = digest.answer(journey, self.headers)?
        {
            journey.body = replay_body.ok_or_else(unreplayable_body)?;
            journey.authorized = Some(authorized);
            return Ok(true);
        }
        Ok(false)
    }
}
