use std::borrow::Cow;

use url::Url;

use crate::FetchSite;
use crate::core::transport::Prepared;

use self::auth::{DigestLeg, unreplayable_body};
use self::journey::{Journey, redirect_location};
use super::Session;
use crate::core::Redirect;
use crate::core::body::Body;
use crate::core::error::{Error, Kind, Result};
use crate::core::request::Draft;
use crate::core::response::{Response, ResponseBody};
use crate::util::without_userinfo;

mod auth;
mod headers;
mod journey;
mod response;

impl Session {
    pub(super) async fn execute_inner(&self, mut attempt: Box<Draft>) -> Result<Response> {
        let url = attempt
            .url
            .take()
            .expect("invariant: send stores the prepared URL before the attempt");
        let headers = std::mem::take(&mut attempt.headers);
        let mut journey = Journey::begin(
            self.hsts_upgrade(&url),
            std::mem::take(&mut attempt.method),
            std::mem::take(&mut attempt.body),
            (!headers.is_empty()).then_some(headers),
            attempt.initiator.take(),
            self.session_referer(),
        )
        .trusting(attempt.trusted_origin.as_ref());
        let digest = attempt.digest_auth.take();
        let Draft {
            preset,
            deadline,
            response,
            body_rules,
            ..
        } = *attempt;
        let redirect_policy = attempt
            .redirect
            .as_ref()
            .unwrap_or(&self.inner.redirect_policy);
        let request_proxy = attempt.proxy.as_ref();
        let header_order = attempt.header_order.as_deref();

        let mut digest = digest.map(DigestLeg::new);
        let redirect_cap = redirect_policy.max_redirects_hint();
        let mut pass = None;
        let mut answered = false;
        let mut refreshed = false;
        loop {
            drop(pass.take());
            pass = self.admit_host(&journey.url).await;
            let refresh = self
                .refresh_version(&journey, digest.as_ref())
                .filter(|_| !refreshed);
            let headers = self.leg_headers(&journey, preset, header_order);
            journey.authorized = None;
            let audit_headers = self.audit_copy(&headers);

            let step_body = std::mem::take(&mut journey.body);
            let replay_body = step_body.replay();

            let proxy = self
                .proxy_for(&journey.url, request_proxy)
                .map_err(leg_error(answered))?;
            let leg = {
                let send = std::pin::pin!(self.send_with_policy(Prepared {
                    method: journey.method.as_str(),
                    url: &journey.url,
                    headers,
                    body: step_body,
                    proxy,
                    response,
                }));
                deadline.response_header(send).await
            }
            .map_err(leg_error(answered))?;
            answered |= leg.status != http::StatusCode::UNAUTHORIZED;
            journey.timing.add_leg(&leg.timing);

            let (mut cookies, mut hsts, mut alt_svc) = (false, false, false);
            for name in leg.headers.keys() {
                cookies |= name == http::header::SET_COOKIE;
                hsts |= name == http::header::STRICT_TRANSPORT_SECURITY;
                alt_svc |= name == http::header::ALT_SVC;
            }
            if cookies {
                self.store_cookies(&leg.headers, &journey.url);
            }
            if hsts {
                self.note_hsts(&journey.url, &leg.headers);
            }
            #[cfg(feature = "http3")]
            if alt_svc {
                self.note_alt_svc(&journey.url, &leg.headers);
            }
            #[cfg(not(feature = "http3"))]
            let _ = alt_svc;

            if leg.status == http::StatusCode::UNAUTHORIZED
                && let Some(version) = refresh
                && replay_body.is_some()
            {
                let token = self
                    .inner
                    .bearer
                    .as_ref()
                    .expect("invariant: refresh version requires bearer auth");
                drop(leg);
                self.unless_shut_down(deadline.total(token.renew(version)))
                    .await?;
                journey.body =
                    replay_body.expect("invariant: replayable body checked before refresh");
                refreshed = true;
                continue;
            }

            let next = Turn {
                code: leg.status.as_u16(),
                headers: &leg.headers,
                redirect_policy,
                redirect_cap,
            };
            if next.take(&mut journey, digest.as_mut(), replay_body)? {
                drop(leg.body);
                journey.url = self.hsts_upgrade(&journey.url);
                continue;
            }

            let mut response = self
                .assemble_response(leg, journey, audit_headers, response, body_rules, &deadline)
                .await?;
            if let ResponseBody::Streaming(body) = &mut response.parts.body {
                body.hold(pass.take());
            }
            return Ok(response);
        }
    }

    pub(crate) fn download_key(&self, draft: &Draft) -> Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let url = draft
            .url
            .as_ref()
            .expect("invariant: download prepares its URL");
        let headers: Vec<_> = draft
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes()))
            .collect();
        let bytes = serde_json::to_vec(&(url.as_str(), headers, self.profile_id()))
            .map_err(Error::from_json)?;
        Ok(Sha256::digest(bytes).into())
    }

    fn session_referer(&self) -> Option<&str> {
        self.inner
            .default_headers
            .iter()
            .rfind(|(k, _)| k.eq_ignore_ascii_case("referer"))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
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

pub(crate) fn url_origin(url: &Url) -> Cow<'_, str> {
    if url.has_authority() && url.username().is_empty() && url.password().is_none() {
        return Cow::Borrowed(&url[..url::Position::BeforePath]);
    }
    let host = url.host_str().unwrap_or("");
    Cow::Owned(match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    })
}

pub(crate) fn referer_for<'a>(prev: Option<&'a str>, current_origin: &str) -> Cow<'a, str> {
    let Some(prev) = prev else {
        return Cow::Owned(format!("{current_origin}/"));
    };
    if prev.strip_suffix('/') == Some(current_origin) {
        return Cow::Borrowed(prev);
    }
    let Ok(parsed) = Url::parse(prev) else {
        return Cow::Owned(format!("{current_origin}/"));
    };
    if parsed.scheme() == "https" && !current_origin.starts_with("https:") {
        return Cow::Borrowed("");
    }
    let origin = url_origin(&parsed);
    if origin != current_origin {
        return Cow::Owned(format!("{origin}/"));
    }
    let mut parsed = without_userinfo(parsed);
    parsed.set_fragment(None);
    Cow::Owned(parsed.to_string())
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
    headers: &'a http::HeaderMap,
    redirect_policy: &'a Redirect,
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
