use http::{HeaderName, HeaderValue};

use super::journey::Journey;
use super::url_origin;
use crate::core::digest::{self, Challenge, DigestAuth};
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::util::request_target;

pub(super) struct DigestLeg {
    auth: DigestAuth,
    answers: u8,
    challenge: Option<Challenge>,
}

impl DigestLeg {
    pub(super) fn new(auth: DigestAuth) -> Self {
        Self {
            auth,
            answers: 0,
            challenge: None,
        }
    }

    pub(super) fn next_hop(&mut self, journey: &Journey) -> Result<Option<HeaderList>> {
        self.answers = 0;
        match &self.challenge {
            Some(challenge)
                if !journey.tainted && in_scope(journey) && challenge.covers(&journey.url) =>
            {
                self.authorized_headers(journey, challenge).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub(super) fn answer(
        &mut self,
        journey: &Journey,
        headers: &[(HeaderName, HeaderValue)],
    ) -> Result<Option<HeaderList>> {
        if !in_scope(journey) {
            return Ok(None);
        }
        let challenges: Vec<Challenge> = headers
            .iter()
            .filter(|(k, _)| *k == "www-authenticate")
            .filter_map(|(_, v)| v.to_str().ok())
            .filter_map(|v| digest::parse_challenge(v).ok())
            .collect();
        let Some(challenge) = challenges
            .iter()
            .find(|c| c.answerable())
            .or(challenges.first())
            .cloned()
        else {
            return Ok(None);
        };
        match (self.answers, challenge.stale) {
            (0, _) | (1, true) => self.answers += 1,
            _ => return Ok(None),
        }
        if self.answers > 1 {
            tracing::debug!(
                target: "leyline::digest",
                nonce = %challenge.nonce,
                "stale nonce — retrying with fresh challenge"
            );
        }
        let authorized = self.authorized_headers(journey, &challenge)?;
        self.challenge = Some(challenge);
        Ok(Some(authorized))
    }

    fn authorized_headers(&self, journey: &Journey, challenge: &Challenge) -> Result<HeaderList> {
        let cnonce = digest::generate_cnonce();
        let nc = digest::next_nc_for_nonce(&challenge.nonce);
        let header = challenge
            .build_auth_header(
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

fn in_scope(journey: &Journey) -> bool {
    journey.chain.is_empty() || url_origin(&journey.url) == journey.original_origin
}

impl Challenge {
    fn answerable(&self) -> bool {
        self.qop
            .as_deref()
            .is_none_or(|qop| digest::pick_supported_qop(qop).is_some())
    }
}

pub(super) fn unreplayable_body() -> Error {
    Error::new(Kind::Request).with_message(
        "digest auth: cannot replay streaming request body. \
         Buffer the body into bytes before sending.",
    )
}
