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
    let mut names: Vec<String> = Vec::new();
    for name in field_names(sent) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| {
            let outcome = header_outcome(&name, sent, observed);
            HeaderOutcome { name, outcome }
        })
        .collect()
}

pub(super) fn order(sent: &[(String, String)], observed: &[(String, String)]) -> FieldOutcome {
    if observed.is_empty() {
        return FieldOutcome::NotReported;
    }
    let expected = shared_names(sent, observed).join(",");
    let seen = shared_names(observed, sent).join(",");
    if expected == seen {
        FieldOutcome::Match
    } else {
        FieldOutcome::Mismatch {
            expected,
            observed: seen,
        }
    }
}

fn field_names(list: &[(String, String)]) -> impl Iterator<Item = String> + '_ {
    list.iter()
        .filter(|(name, _)| !name.starts_with(':'))
        .map(|(name, _)| name.to_ascii_lowercase())
}

fn shared_names(from: &[(String, String)], other: &[(String, String)]) -> Vec<String> {
    field_names(from)
        .filter(|name| {
            other
                .iter()
                .any(|(held, _)| held.eq_ignore_ascii_case(name))
        })
        .collect()
}

fn values<'a>(name: &str, list: &'a [(String, String)]) -> Vec<&'a str> {
    list.iter()
        .filter(|(held, _)| held.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .collect()
}

fn header_outcome(
    name: &str,
    sent: &[(String, String)],
    observed: &[(String, String)],
) -> FieldOutcome {
    if observed.is_empty() {
        return FieldOutcome::NotReported;
    }
    let expected = values(name, sent);
    let seen = values(name, observed);
    if seen.is_empty() {
        return FieldOutcome::Absent {
            expected: expected.join(", "),
        };
    }
    if seen == expected {
        return FieldOutcome::Match;
    }
    FieldOutcome::Mismatch {
        expected: expected.join(", "),
        observed: seen.join(", "),
    }
}
