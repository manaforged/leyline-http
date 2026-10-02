use std::collections::HashSet;
use std::fmt::Write;

use super::HeaderStyle;

const Q_START_TENTHS: u8 = 10;
const Q_STEP_TENTHS: u8 = 1;
const Q_FLOOR_TENTHS: u8 = 1;
const PRIVATE_BASES: [&str; 2] = ["x", "i"];
const SUBTAG_MAX: usize = 8;
const PRIMARY_MIN: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LanguageRule {
    expand_base: bool,
    canonical_case: bool,
    limit: Option<usize>,
}

const CHROMIUM: LanguageRule = LanguageRule {
    expand_base: true,
    canonical_case: false,
    limit: None,
};

const GECKO: LanguageRule = LanguageRule {
    expand_base: false,
    canonical_case: true,
    limit: None,
};

const WEBKIT: LanguageRule = LanguageRule {
    expand_base: false,
    canonical_case: false,
    limit: Some(1),
};

const RULES: [(HeaderStyle, LanguageRule); 4] = [
    (HeaderStyle::Gecko, GECKO),
    (HeaderStyle::WebKit, WEBKIT),
    (HeaderStyle::WebKit26, WEBKIT),
    (HeaderStyle::WebKit17, WEBKIT),
];

fn rule_for(style: HeaderStyle) -> LanguageRule {
    RULES
        .iter()
        .find(|(listed, _)| *listed == style)
        .map_or(CHROMIUM, |(_, rule)| *rule)
}

pub(crate) fn validate_language(tag: &str) -> Result<(), String> {
    let mut subtags = tag.split('-');
    let primary = subtags.next().unwrap_or_default();
    let primary_ok = ((PRIMARY_MIN..=SUBTAG_MAX).contains(&primary.len())
        && primary.bytes().all(|b| b.is_ascii_alphabetic()))
        || PRIVATE_BASES
            .iter()
            .any(|base| primary.eq_ignore_ascii_case(base));
    let rest_ok = subtags.all(|subtag| {
        (1..=SUBTAG_MAX).contains(&subtag.len())
            && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    if primary_ok && rest_ok {
        Ok(())
    } else {
        Err(format!(
            "invalid language tag `{}`: expected a BCP 47 tag such as `de-DE`",
            tag.escape_debug()
        ))
    }
}

pub(crate) fn accept_language(tags: &[String], style: HeaderStyle) -> String {
    let rule = rule_for(style);
    let limit = rule.limit.unwrap_or(usize::MAX);
    let listed: Vec<String> = tags
        .iter()
        .take(limit)
        .map(|tag| cased(tag, rule.canonical_case))
        .collect();
    let ordered = if rule.expand_base {
        expand_bases(&listed)
    } else {
        listed
    };
    weighted(&ordered)
}

fn base_of(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

fn expand_bases(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |tag: &str, out: &mut Vec<String>| {
        if seen.insert(tag.to_owned()) {
            out.push(tag.to_owned());
        }
    };
    for (index, tag) in tags.iter().enumerate() {
        push(tag, &mut out);
        let base = base_of(tag);
        if PRIVATE_BASES.contains(&base) {
            continue;
        }
        if tags.get(index + 1).is_none_or(|next| base_of(next) != base) {
            push(base, &mut out);
        }
    }
    out
}

fn weighted(tags: &[String]) -> String {
    let mut q = Q_START_TENTHS;
    let mut out = String::new();
    for tag in tags {
        if q == Q_START_TENTHS {
            out.push_str(tag);
        } else {
            let _ = write!(out, ",{tag};q=0.{q}");
        }
        if q > Q_STEP_TENTHS.max(Q_FLOOR_TENTHS) {
            q -= Q_STEP_TENTHS;
        }
    }
    out
}

fn cased(tag: &str, canonical: bool) -> String {
    if !canonical {
        return tag.to_owned();
    }
    let lower = tag.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut singleton = false;
    for (index, subtag) in lower.split('-').enumerate() {
        singleton |= index > 0 && subtag.len() == 1;
        out.push(if index == 0 || singleton {
            subtag.to_owned()
        } else {
            canonical_subtag(subtag)
        });
    }
    out.join("-")
}

fn canonical_subtag(subtag: &str) -> String {
    match subtag.len() {
        2 => subtag.to_ascii_uppercase(),
        4 => {
            let (head, tail) = subtag.split_at(1);
            format!("{}{tail}", head.to_ascii_uppercase())
        }
        _ => subtag.to_owned(),
    }
}
