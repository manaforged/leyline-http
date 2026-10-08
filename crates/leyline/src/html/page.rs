use super::Anchor;
use super::build::collapse;
use super::entity::decode;
use super::scan::{Scanner, Tag};

const META_KEYS: [&str; 2] = ["name", "property"];

pub(super) fn meta(document: &str, name: &str) -> Option<String> {
    Scanner::new(document)
        .filter(|tag| !tag.closing && tag.name == "meta")
        .find(|tag| tag.names(name))
        .and_then(|tag| tag.attr("content").map(str::to_owned))
}

impl Tag<'_> {
    fn names(&self, name: &str) -> bool {
        META_KEYS
            .iter()
            .filter_map(|key| self.attr(key))
            .any(|value| value.trim().eq_ignore_ascii_case(name))
    }
}

struct Open {
    href: String,
    rel: Option<String>,
    text: String,
}

pub(super) fn links(document: &str) -> Vec<Anchor> {
    let mut done = Vec::new();
    let mut open: Option<Open> = None;
    for tag in Scanner::new(document) {
        if tag.name == "a" {
            done.extend(open.take().map(finish));
            open = (!tag.closing).then(|| start(&tag)).flatten();
        }
        if let Some(anchor) = open.as_mut() {
            anchor.text.push_str(tag.text);
        }
    }
    done.extend(open.map(finish));
    done
}

fn start(tag: &Tag<'_>) -> Option<Open> {
    Some(Open {
        href: tag.attr("href")?.trim().to_owned(),
        rel: tag.attr("rel").map(str::to_owned),
        text: String::new(),
    })
}

fn finish(open: Open) -> Anchor {
    Anchor {
        href: open.href,
        text: collapse(&decode(&open.text)),
        rel: open.rel,
    }
}
