use serde::Deserialize;

use super::FieldOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeaderOutcome {
    pub name: String,
    pub outcome: FieldOutcome,
}

#[derive(Deserialize, Default)]
pub(super) struct EchoHttp1 {
    #[serde(default)]
    headers: Vec<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct EchoFrame {
    #[serde(default)]
    headers: Vec<String>,
}

pub(super) fn echoed(http2: &[EchoFrame], http1: &EchoHttp1) -> Vec<(String, String)> {
    http2
        .iter()
        .flat_map(|frame| frame.headers.iter())
        .chain(http1.headers.iter())
        .filter_map(|line| split_line(line))
        .collect()
}

fn split_line(line: &str) -> Option<(String, String)> {
    if line.starts_with(':') {
        return None;
    }
    let (name, value) = line.split_once(':')?;
    let name = name.trim();
    (!name.is_empty()).then(|| (name.to_ascii_lowercase(), value.trim().to_owned()))
}

pub(super) fn compare(
    sent: &[(String, String)],
    observed: &[(String, String)],
) -> Vec<HeaderOutcome> {
    sent.iter()
        .filter(|(name, _)| !name.starts_with(':'))
        .map(|(name, value)| HeaderOutcome {
            name: name.to_ascii_lowercase(),
            outcome: header_outcome(name, value, observed),
        })
        .collect()
}

fn header_outcome(name: &str, expected: &str, observed: &[(String, String)]) -> FieldOutcome {
    let mut seen = observed
        .iter()
        .filter(|(held, _)| held.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .peekable();
    let Some(first) = seen.peek().copied() else {
        return FieldOutcome::NotReported;
    };
    if seen.any(|value| value == expected.trim()) {
        return FieldOutcome::Match;
    }
    FieldOutcome::Mismatch {
        expected: expected.to_owned(),
        observed: first.to_owned(),
    }
}
