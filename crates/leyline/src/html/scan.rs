use super::entity::decode_attribute;

const RAW_TEXT: [&str; 4] = ["script", "style", "textarea", "title"];

const EMPTY_COMMENTS: [&str; 2] = [">", "->"];

pub(super) struct Tag<'a> {
    pub(super) name: String,
    pub(super) closing: bool,
    pub(super) attrs: Vec<(String, String)>,
    pub(super) raw: Option<&'a str>,
    pub(super) text: &'a str,
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
}

impl<'a> Scanner<'a> {
    pub(super) fn new(doc: &'a str) -> Self {
        Self { doc, pos: 0 }
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

    fn markup(&mut self, after: usize) -> bool {
        let rest = &self.doc[after..];
        if let Some(body) = rest.strip_prefix("!--") {
            match EMPTY_COMMENTS.iter().find(|end| body.starts_with(**end)) {
                Some(end) => self.pos = after + 3 + end.len(),
                None => self.skip_past(after + 3, "-->"),
            }
            return true;
        }
        if rest.starts_with('!') || rest.starts_with('?') {
            self.skip_past(after, ">");
            return true;
        }
        false
    }
}

impl<'a> Iterator for Scanner<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        loop {
            let at = self.pos + self.doc[self.pos..].find('<')?;
            self.pos = at + 1;
            if self.markup(self.pos) {
                continue;
            }
            let closing = self.doc[self.pos..].starts_with('/');
            let start = self.pos + usize::from(closing);
            let bytes = &self.doc.as_bytes()[start..];
            if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            let len = bytes
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-')
                .count();
            let name = self.doc[start..start + len].to_ascii_lowercase();
            let (attrs, end) = attributes(self.doc, start + len);
            self.pos = end;
            let raw = (!closing && RAW_TEXT.contains(&name.as_str())).then(|| self.raw_text(&name));
            let text = self.doc[self.pos..].split('<').next().unwrap_or_default();
            return Some(Tag {
                name,
                closing,
                attrs,
                raw,
                text,
            });
        }
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
