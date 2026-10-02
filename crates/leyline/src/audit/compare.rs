use serde::Deserialize;

use crate::audit::AuditData;
use crate::core::{Error, Result};

mod headers;

pub use headers::HeaderOutcome;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Observed {
    pub ja4: Option<String>,
    pub ja3: Option<String>,
    pub h2_fingerprint: Option<String>,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldOutcome {
    Match,
    Mismatch { expected: String, observed: String },
    Informational { expected: String, observed: String },
    NotReported,
    Absent { expected: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FingerprintReport {
    pub ja4: FieldOutcome,
    pub ja3: FieldOutcome,
    pub h2_fingerprint: FieldOutcome,
    pub headers: Vec<HeaderOutcome>,
    pub header_order: FieldOutcome,
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

impl Observed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ja4(mut self, value: impl Into<String>) -> Self {
        self.ja4 = Some(value.into());
        self
    }

    pub fn ja3(mut self, value: impl Into<String>) -> Self {
        self.ja3 = Some(value.into());
        self
    }

    pub fn h2_fingerprint(mut self, value: impl Into<String>) -> Self {
        self.h2_fingerprint = Some(value.into());
        self
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    pub fn from_json(text: &str) -> Result<Observed> {
        let echo: EchoAll = serde_json::from_str(text).map_err(Error::from_json)?;
        Ok(Observed {
            headers: headers::echoed(&echo.http2.sent_frames, &echo.http1),
            ja4: echo.tls.ja4,
            ja3: echo.tls.ja3_hash,
            h2_fingerprint: echo.http2.akamai_fingerprint,
        })
    }
}

impl FieldOutcome {
    pub fn is_mismatch(&self) -> bool {
        matches!(
            self,
            FieldOutcome::Mismatch { .. } | FieldOutcome::Absent { .. }
        )
    }
}

impl FingerprintReport {
    pub fn is_match(&self) -> bool {
        [
            &self.ja4,
            &self.ja3,
            &self.h2_fingerprint,
            &self.header_order,
        ]
        .iter()
        .all(|outcome| !outcome.is_mismatch())
            && self.headers.iter().all(|h| !h.outcome.is_mismatch())
    }
}

impl AuditData {
    pub fn compare(&self, observed: &Observed) -> FingerprintReport {
        FingerprintReport {
            ja4: outcome(&self.ja4, observed.ja4.as_deref(), false),
            ja3: outcome(&self.ja3, observed.ja3.as_deref(), self.permutes_extensions),
            h2_fingerprint: outcome(
                &self.h2_fingerprint,
                observed.h2_fingerprint.as_deref(),
                false,
            ),
            headers: headers::compare(&self.request_headers, &observed.headers),
            header_order: headers::order(&self.request_headers, &observed.headers),
        }
    }
}

fn outcome(expected: &str, observed: Option<&str>, order_varies: bool) -> FieldOutcome {
    let Some(observed) = observed else {
        return FieldOutcome::NotReported;
    };
    if expected.eq_ignore_ascii_case(observed.trim()) {
        return FieldOutcome::Match;
    }
    let expected = expected.to_owned();
    let observed = observed.to_owned();
    if order_varies {
        FieldOutcome::Informational { expected, observed }
    } else {
        FieldOutcome::Mismatch { expected, observed }
    }
}

impl std::fmt::Display for FieldOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldOutcome::Match => f.write_str("match"),
            FieldOutcome::Mismatch { expected, observed } => {
                write!(f, "mismatch (expected {expected}, observed {observed})")
            }
            FieldOutcome::Informational { expected, observed } => {
                write!(
                    f,
                    "informational (expected {expected}, observed {observed})"
                )
            }
            FieldOutcome::NotReported => f.write_str("not reported"),
            FieldOutcome::Absent { expected } => write!(f, "absent (expected {expected})"),
        }
    }
}

impl std::fmt::Display for FingerprintReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ja4: {}", self.ja4)?;
        writeln!(f, "ja3: {}", self.ja3)?;
        write!(f, "h2: {}", self.h2_fingerprint)?;
        write!(f, "\nheader order: {}", self.header_order)?;
        for header in &self.headers {
            write!(f, "\nheader {}: {}", header.name, header.outcome)?;
        }
        Ok(())
    }
}
