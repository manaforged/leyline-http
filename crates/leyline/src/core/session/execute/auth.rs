use http::{HeaderName, HeaderValue};

use super::journey::Journey;
use super::url_origin;
use crate::core::digest::{self, Challenge, DigestAuth};
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::util::request_target;

pub(super) struct DigestLeg {
    auth: DigestAuth,
    answered: bool,
    stale_retried: bool,
}

impl DigestLeg {
    pub(super) fn new(auth: DigestAuth) -> Self {
        Self {
            auth,
            answered: false,
            stale_retried: false,
        }
    }

    pub(super) fn next_hop(&mut self) {
        self.answered = false;
        self.stale_retried = false;
    }

    pub(super) fn answer(
        &mut self,
        journey: &Journey,
        headers: &[(HeaderName, HeaderValue)],
    ) -> Result<Option<HeaderList>> {
        if !journey.chain.is_empty() && url_origin(&journey.url) != journey.original_origin {
            return Ok(None);
        }
        let Some(challenge) = headers
            .iter()
            .filter(|(k, _)| *k == "www-authenticate")
            .filter_map(|(_, v)| v.to_str().ok())
            .find_map(|v| digest::parse_challenge(v).ok())
        else {
            return Ok(None);
        };
        if !self.answered {
            self.answered = true;
        } else if !self.stale_retried && challenge.stale {
            tracing::debug!(
                target: "leyline::digest",
                nonce = %challenge.nonce,
                "stale nonce — retrying with fresh challenge"
            );
            self.stale_retried = true;
        } else {
            return Ok(None);
        }
        self.authorized_headers(journey, &challenge).map(Some)
    }

    fn authorized_headers(&self, journey: &Journey, challenge: &Challenge) -> Result<HeaderList> {
        let cnonce = digest::generate_cnonce();
        let nc = digest::next_nc_for_nonce(&challenge.nonce);
        let header = digest::build_auth_header(
            challenge,
            &self.auth,
            &journey.method,
            request_target(&journey.url),
            nc,
            &cnonce,
        )
        .ok_or_else(|| {
            Error::new(Kind::Request).with_message(
                "digest auth: server offered only qop=auth-int, \
                 which Leyline does not implement (RFC 7616 §3.4.3 \
                 requires the entity-body hash in HA2). Pass through \
                 the 401 or remove digest_auth().",
            )
        })?;
        let mut headers = journey.extra.clone().unwrap_or_default();
        headers.set("authorization", header)?;
        Ok(headers)
    }
}

pub(super) fn unreplayable_body() -> Error {
    Error::new(Kind::Request).with_message(
        "digest auth: cannot replay streaming request body. \
         Buffer the body into bytes before sending.",
    )
}
