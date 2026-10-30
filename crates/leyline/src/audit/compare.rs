use serde::Deserialize;

use crate::audit::Fingerprint;
use crate::core::{Error, Result};

mod headers;

pub(crate) use headers::HeaderOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldOutcome {
    Match,
    Mismatch { expected: String, observed: String },
    Informational,
    NotReported,
    Absent { expected: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FingerprintReport {
    pub(crate) ja4: FieldOutcome,
    pub(crate) ja3: FieldOutcome,
    pub(crate) h2_fingerprint: FieldOutcome,
    pub(crate) headers: Vec<HeaderOutcome>,
    pub(crate) header_order: FieldOutcome,
}

#[derive(Deserialize, Default)]
struct EchoAll {
    #[serde(default)]
    tls: EchoTls,
    #[serde(default)]
    http2: EchoHttp2,
    #[serde(default)]
    http1: headers::EchoHttp1,
}

#[derive(Deserialize, Default)]
struct EchoTls {
    ja4: Option<String>,
    ja3_hash: Option<String>,
}

#[derive(Deserialize, Default)]
struct EchoHttp2 {
    akamai_fingerprint: Option<String>,
    #[serde(default)]
    sent_frames: Vec<headers::EchoFrame>,
}

impl FieldOutcome {
    fn mismatch(&self) -> Option<(String, String)> {
        match self {
            FieldOutcome::Mismatch { expected, observed } => {
                Some((expected.clone(), observed.clone()))
            }
            FieldOutcome::Absent { expected } => Some((expected.clone(), String::new())),
            _ => None,
        }
    }
}

impl Fingerprint {
    /// Parses an echo service's JSON report into the fingerprint it observed.
    pub fn from_json(text: &str) -> Result<Fingerprint> {
        let echo: EchoAll = serde_json::from_str(text).map_err(Error::from_json)?;
        Ok(Fingerprint {
            request_headers: headers::echoed(&echo.http2.sent_frames, &echo.http1),
            ja4: echo.tls.ja4.unwrap_or_default(),
            ja3: echo.tls.ja3_hash.unwrap_or_default(),
            h2_fingerprint: echo.http2.akamai_fingerprint.unwrap_or_default(),
            ..Fingerprint::default()
        })
    }

    /// Returns one `(field, expected, observed)` entry per mismatched field, and an empty list when the fingerprints match.
    pub fn diff(&self, observed: &Fingerprint) -> Vec<(String, String, String)> {
        let report = self.compare(observed);
        [
            ("ja4", &report.ja4),
            ("ja3", &report.ja3),
            ("akamai", &report.h2_fingerprint),
            ("header order", &report.header_order),
        ]
        .into_iter()
        .map(|(name, outcome)| (name.to_owned(), outcome))
        .chain(
            report
                .headers
                .iter()
                .map(|header| (header.name.clone(), &header.outcome)),
        )
        .filter_map(|(name, outcome)| {
            outcome
                .mismatch()
                .map(|(expected, seen)| (name, expected, seen))
        })
        .collect()
    }

    pub(crate) fn compare(&self, observed: &Fingerprint) -> FingerprintReport {
        FingerprintReport {
            ja4: outcome(&self.ja4, &observed.ja4, false),
            ja3: outcome(&self.ja3, &observed.ja3, self.permutes_extensions),
            h2_fingerprint: outcome(&self.h2_fingerprint, &observed.h2_fingerprint, false),
            headers: headers::compare(&self.request_headers, &observed.request_headers),
            header_order: headers::order(&self.request_headers, &observed.request_headers),
        }
    }
}

fn outcome(expected: &str, observed: &str, order_varies: bool) -> FieldOutcome {
    if observed.is_empty() {
        return FieldOutcome::NotReported;
    }
    if expected.eq_ignore_ascii_case(observed.trim()) {
        return FieldOutcome::Match;
    }
    if order_varies {
        FieldOutcome::Informational
    } else {
        FieldOutcome::Mismatch {
            expected: expected.to_owned(),
            observed: observed.to_owned(),
        }
    }
}
