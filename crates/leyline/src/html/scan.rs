use super::entity::decode_attribute;

const RAW_TEXT: [&str; 4] = ["script", "style", "textarea", "title"];

const FOREIGN: [&str; 2] = ["svg", "math"];

const EMPTY_COMMENTS: [&str; 2] = [">", "->"];
const COMMENT_OPEN: &str = "!--";
const COMMENT_CLOSE: &str = "-->";
const CDATA_OPEN: &str = "![CDATA[";
const CDATA_CLOSE: &str = "]]>";

pub(super) struct Tag<'a> {
    pub(super) name: String,
    pub(super) closing: bool,
    pub(super) self_closing: bool,
    pub(super) attrs: Vec<(String, String)>,
    pub(super) raw: Option<&'a str>,
    pub(super) text: String,
}

impl Tag<'_> {
    pub(super) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn has(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }
}

pub(super) struct Scanner<'a> {
    doc: &'a str,
    pos: usize,
    started: bool,
    xml: bool,
}

impl<'a> Scanner<'a> {
    pub(super) fn new(doc: &'a str) -> Self {
        Self {
            doc,
            pos: 0,
            started: false,
            xml: false,
        }
    }

    pub(super) fn xml(doc: &'a str) -> Self {
        Self {
            xml: true,
            ..Self::new(doc)
        }
    }

    fn skip_past(&mut self, from: usize, end: &str) {
        self.pos = self.doc[from..]
            .find(end)
            .map_or(self.doc.len(), |at| from + at + end.len());
    }

    fn raw_text(&mut self, name: &str) -> &'a str {
        let doc = self.doc;
        let start = self.pos;
        let mut from = start;
        while let Some(found) = doc[from..].find("</") {
            let at = from + found;
            let tail = &doc.as_bytes()[at + 2..];
            if tail.len() > name.len()
                && tail[..name.len()].eq_ignore_ascii_case(name.as_bytes())
                && ends_name(tail[name.len()])
            {
                self.skip_past(at, ">");
                return &doc[start..at];
            }
            from = at + 2;
        }
        self.pos = doc.len();
        &doc[start..]
    }

    fn markup(&mut self, open: usize) -> Markup<'a> {
        let doc = self.doc;
        let after = open + 1;
        let rest = &doc[after..];
        if let Some(body) = rest.strip_prefix(COMMENT_OPEN) {
            let from = after + COMMENT_OPEN.len();
            match EMPTY_COMMENTS.iter().find(|end| body.starts_with(**end)) {
                Some(end) => self.pos = from + end.len(),
                None => self.skip_past(from, COMMENT_CLOSE),
            }
            return Markup::Skipped;
        }
        if self.xml && rest.starts_with(CDATA_OPEN) {
            let from = after + CDATA_OPEN.len();
            let to = doc[from..]
                .find(CDATA_CLOSE)
                .map_or(doc.len(), |at| from + at);
            self.skip_past(to, CDATA_CLOSE);
            return Markup::Cdata(&doc[from..to]);
        }
        if rest.starts_with('!') || rest.starts_with('?') {
            self.skip_past(after, ">");
            return Markup::Skipped;
        }
        let closing = rest.starts_with('/');
        let named = rest.as_bytes()[usize::from(closing)..]
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
        match (named, closing) {
            (true, _) => {
                self.pos = open;
                Markup::Tag
            }
            (false, true) => {
                self.skip_past(after, ">");
                Markup::Skipped
            }
            (false, false) => {
                self.pos = after;
                Markup::Literal
            }
        }
    }

    fn gather(&mut self) -> String {
        let mut text = String::new();
        loop {
            let rest = &self.doc[self.pos..];
            let Some(at) = rest.find('<') else {
                text.push_str(rest);
                self.pos = self.doc.len();
                return text;
            };
            text.push_str(&rest[..at]);
            match self.markup(self.pos + at) {
                Markup::Tag => return text,
                Markup::Literal => text.push('<'),
                Markup::Skipped => {}
                Markup::Cdata(body) => escape(body, &mut text),
            }
        }
    }

    pub(super) fn lead(&mut self) -> String {
        self.started = true;
        self.gather()
    }
}

enum Markup<'a> {
    Tag,
    Literal,
    Skipped,
    Cdata(&'a str),
}

fn escape(body: &str, out: &mut String) {
    for ch in body.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            other => out.push(other),
        }
    }
}

impl<'a> Iterator for Scanner<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        if !self.started {
            self.lead();
        }
        if self.pos >= self.doc.len() {
            return None;
        }
        let closing = self.doc[self.pos + 1..].starts_with('/');
        let start = self.pos + 1 + usize::from(closing);
        let len = self.doc.as_bytes()[start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':'))
            .count();
        let name = self.doc[start..start + len].to_ascii_lowercase();
        let (attrs, end) = attributes(self.doc, start + len);
        let self_closing =
            FOREIGN.iter().any(|foreign| *foreign == name) && self.doc[..end].ends_with("/>");
        self.pos = end;
        let raw =
            (!closing && RAW_TEXT.iter().any(|raw| *raw == name)).then(|| self.raw_text(&name));
        let text = self.gather();
        Some(Tag {
            name,
            closing,
            self_closing,
            attrs,
            raw,
            text,
        })
    }
}

fn attributes(doc: &str, from: usize) -> (Vec<(String, String)>, usize) {
    let bytes = doc.as_bytes();
    let mut attrs = Vec::new();
    let mut i = from;
    loop {
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'/')
        {
            i += 1;
        }
        match bytes.get(i) {
            None => return (attrs, doc.len()),
            Some(b'>') => return (attrs, i + 1),
            Some(_) => {}
        }
        let start = i;
        i += 1;
        while bytes
            .get(i)
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'/'))
        {
            i += 1;
        }
        let name = doc[start..i].to_ascii_lowercase();
        let (value, next) = value(doc, i);
        i = next;
        attrs.push((name, value));
    }
}

fn value(doc: &str, from: usize) -> (String, usize) {
    let bytes = doc.as_bytes();
    let mut i = from;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'=') {
        return (String::new(), from);
    }
    i += 1;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    match bytes.get(i) {
        Some(&quote @ (b'"' | b'\'')) => {
            let start = i + 1;
            let end = doc[start..]
                .find(char::from(quote))
                .map_or(doc.len(), |at| start + at);
            (decode_attribute(&doc[start..end]), (end + 1).min(doc.len()))
        }
        _ => {
            let start = i;
            while bytes
                .get(i)
                .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
            {
                i += 1;
            }
            (decode_attribute(&doc[start..i]), i)
        }
    }
}

fn ends_name(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>')
}
