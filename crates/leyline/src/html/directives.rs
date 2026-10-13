use http::HeaderMap;

use super::page::Element;
use super::scan::Scanner;
use super::{Directives, lookup};
use crate::core::product_token;

const META_NAME: &str = "robots";
const HEADER: &str = "x-robots-tag";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Directive {
    NoIndex,
    NoFollow,
    Nothing,
    NoAi,
    Other,
}

const DIRECTIVES: [(&str, Directive); 17] = [
    ("noindex", Directive::NoIndex),
    ("nofollow", Directive::NoFollow),
    ("none", Directive::Nothing),
    ("noai", Directive::NoAi),
    ("all", Directive::Other),
    ("index", Directive::Other),
    ("follow", Directive::Other),
    ("noarchive", Directive::Other),
    ("nosnippet", Directive::Other),
    ("notranslate", Directive::Other),
    ("noimageindex", Directive::Other),
    ("noimageai", Directive::Other),
    ("indexifembedded", Directive::Other),
    ("unavailable_after", Directive::Other),
    ("max-snippet", Directive::Other),
    ("max-image-preview", Directive::Other),
    ("max-video-preview", Directive::Other),
];

impl Directive {
    fn parse(token: &str) -> Option<Directive> {
        lookup(&DIRECTIVES, token)
    }
}

pub(super) fn read(document: &str, headers: &HeaderMap, agent: &str) -> Directives {
    let mut found = Directives {
        index: true,
        follow: true,
        ai: true,
    };
    for tag in Scanner::new(document) {
        if tag.closing || Element::of(&tag.name) != Element::Meta {
            continue;
        }
        let names_us = tag
            .attr("name")
            .map(str::trim)
            .is_some_and(|name| name.eq_ignore_ascii_case(META_NAME) || names_agent(name, agent));
        if let (true, Some(content)) = (names_us, tag.attr("content")) {
            apply(&mut found, content);
        }
    }
    for value in headers
        .get_all(HEADER)
        .iter()
        .filter_map(|v| v.to_str().ok())
    {
        let (scope, list) = scoped(value);
        if scope.is_none_or(|scope| names_agent(scope, agent)) {
            apply(&mut found, list);
        }
    }
    found
}

fn names_agent(name: &str, agent: &str) -> bool {
    let wanted = product_token(agent);
    !wanted.is_empty() && product_token(name).eq_ignore_ascii_case(wanted)
}

fn scoped(value: &str) -> (Option<&str>, &str) {
    match value.split_once(':') {
        Some((head, rest)) if !head.contains(',') && Directive::parse(head.trim()).is_none() => {
            (Some(head.trim()), rest)
        }
        _ => (None, value),
    }
}

fn apply(found: &mut Directives, list: &str) {
    for name in list
        .split(',')
        .filter_map(|item| item.split(':').next())
        .flat_map(str::split_whitespace)
    {
        match Directive::parse(name) {
            Some(Directive::NoIndex) => found.index = false,
            Some(Directive::NoFollow) => found.follow = false,
            Some(Directive::Nothing) => {
                found.index = false;
                found.follow = false;
            }
            Some(Directive::NoAi) => found.ai = false,
            Some(Directive::Other) | None => {}
        }
    }
}
