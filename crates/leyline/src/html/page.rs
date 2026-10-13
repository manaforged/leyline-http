use super::Anchor;
use super::build::collapse;
use super::entity::decode;
use super::scan::{Scanner, Tag};

const META_KEYS: [&str; 2] = ["name", "property"];
const CANONICAL: &str = "canonical";

pub(super) fn meta(document: &str, name: &str) -> Option<String> {
    Scanner::new(document)
        .filter(|tag| !tag.closing && tag.name == "meta")
        .find(|tag| tag.names(name))
        .and_then(|tag| tag.attr("content").map(str::to_owned))
}

pub(super) fn base(document: &str) -> Option<String> {
    Scanner::new(document)
        .filter(|tag| !tag.closing && Element::of(&tag.name) == Element::Base)
        .find_map(|tag| tag.attr("href").map(|href| href.trim().to_owned()))
}

pub(super) fn canonical(document: &str) -> Option<String> {
    Scanner::new(document)
        .filter(|tag| !tag.closing && Element::of(&tag.name) == Element::Link)
        .find(|tag| {
            tag.attr("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case(CANONICAL))
            })
        })
        .and_then(|tag| tag.attr("href").map(|href| href.trim().to_owned()))
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
            anchor.text.push_str(&tag.text);
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Element {
    Inline,
    Block,
    Body,
    Pre,
    Head,
    HeadContent,
    Link,
    Base,
    Meta,
    Hidden,
}

const ELEMENTS: &[(&str, Element)] = &[
    ("head", Element::Head),
    ("title", Element::HeadContent),
    ("style", Element::HeadContent),
    ("script", Element::HeadContent),
    ("base", Element::Base),
    ("link", Element::Link),
    ("meta", Element::Meta),
    ("noscript", Element::Hidden),
    ("template", Element::Hidden),
    ("svg", Element::Hidden),
    ("body", Element::Body),
    ("pre", Element::Pre),
    ("address", Element::Block),
    ("article", Element::Block),
    ("aside", Element::Block),
    ("blockquote", Element::Block),
    ("br", Element::Block),
    ("caption", Element::Block),
    ("dd", Element::Block),
    ("details", Element::Block),
    ("dialog", Element::Block),
    ("div", Element::Block),
    ("dl", Element::Block),
    ("dt", Element::Block),
    ("fieldset", Element::Block),
    ("figcaption", Element::Block),
    ("figure", Element::Block),
    ("footer", Element::Block),
    ("form", Element::Block),
    ("h1", Element::Block),
    ("h2", Element::Block),
    ("h3", Element::Block),
    ("h4", Element::Block),
    ("h5", Element::Block),
    ("h6", Element::Block),
    ("header", Element::Block),
    ("hr", Element::Block),
    ("li", Element::Block),
    ("main", Element::Block),
    ("nav", Element::Block),
    ("ol", Element::Block),
    ("p", Element::Block),
    ("section", Element::Block),
    ("summary", Element::Block),
    ("table", Element::Block),
    ("td", Element::Block),
    ("th", Element::Block),
    ("tr", Element::Block),
    ("ul", Element::Block),
];

impl Element {
    pub(super) fn of(name: &str) -> Element {
        ELEMENTS
            .iter()
            .find(|(element, _)| *element == name)
            .map_or(Element::Inline, |(_, kind)| *kind)
    }

    fn ends_head(self) -> bool {
        matches!(
            self,
            Element::Inline | Element::Block | Element::Body | Element::Pre
        )
    }
}

#[derive(Default)]
struct Text {
    lines: Vec<String>,
    block: String,
    hidden: usize,
    in_head: bool,
    pre: bool,
}

impl Text {
    fn tag(&mut self, tag: &Tag<'_>) {
        let element = Element::of(&tag.name);
        if !tag.closing && element.ends_head() {
            self.in_head = false;
        }
        match element {
            Element::Head => self.in_head = !tag.closing,
            Element::Block | Element::Body => self.flush(),
            Element::Pre => {
                self.flush();
                self.pre = !tag.closing;
            }
            Element::Hidden if tag.self_closing => {}
            Element::Hidden if tag.closing => self.hidden = self.hidden.saturating_sub(1),
            Element::Hidden => self.hidden += 1,
            Element::Inline
            | Element::HeadContent
            | Element::Link
            | Element::Base
            | Element::Meta => {}
        }
        if self.hidden == 0 && !self.in_head {
            self.block.push_str(&tag.text);
        }
    }

    fn flush(&mut self) {
        let raw = decode(&std::mem::take(&mut self.block));
        let line = if self.pre {
            raw.strip_prefix('\n').unwrap_or(&raw).trim_end().to_owned()
        } else {
            collapse(&raw)
        };
        if !line.trim().is_empty() {
            self.lines.push(line);
        }
    }
}

pub(super) fn text(document: &str) -> String {
    let mut text = Text::default();
    let mut scanner = Scanner::new(document);
    text.block = scanner.lead();
    for tag in scanner {
        text.tag(&tag);
    }
    text.flush();
    text.lines.join("\n")
}
