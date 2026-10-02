use http::header::LINK;
use url::Url;

use super::Response;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Link {
    pub url: Url,
    pub rel: Vec<String>,
    pub params: Vec<(String, String)>,
}

impl Link {
    pub fn has_rel(&self, rel: &str) -> bool {
        self.rel.iter().any(|r| r.eq_ignore_ascii_case(rel))
    }
}

impl Response {
    pub fn links(&self) -> Vec<Link> {
        let mut out = Vec::new();
        for value in self.headers.get_all(LINK) {
            if let Ok(text) = value.to_str() {
                Parser::new(text).parse_into(&self.url, &mut out);
            }
        }
        out
    }

    pub fn link(&self, rel: &str) -> Option<Url> {
        self.links()
            .into_iter()
            .find(|link| link.has_rel(rel))
            .map(|link| link.url)
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_ws();
        let found = self.peek() == Some(byte);
        if found {
            self.pos += 1;
        }
        found
    }

    fn parse_into(mut self, base: &Url, out: &mut Vec<Link>) {
        loop {
            while self.eat(b',') {}
            if self.peek().is_none() {
                return;
            }
            match self.link_value(base) {
                Some(link) => out.push(link),
                None => self.skip_value(),
            }
        }
    }

    fn link_value(&mut self, base: &Url) -> Option<Link> {
        if !self.eat(b'<') {
            return None;
        }
        let end = self.src[self.pos..].find('>')? + self.pos;
        let target = &self.src[self.pos..end];
        self.pos = end + 1;
        let mut rel = None;
        let mut params = Vec::new();
        while self.eat(b';') {
            let Some((name, value)) = self.param() else {
                continue;
            };
            if name.eq_ignore_ascii_case("rel") {
                rel.get_or_insert(value);
            } else {
                params.push((name, value));
            }
        }
        let url = base.join(target.trim()).ok()?;
        let rel = rel
            .map(|r| r.split_ascii_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        Some(Link { url, rel, params })
    }

    fn param(&mut self) -> Option<(String, String)> {
        self.skip_ws();
        let name = self.token();
        if name.is_empty() {
            return None;
        }
        if !self.eat(b'=') {
            return Some((name.to_ascii_lowercase(), String::new()));
        }
        self.skip_ws();
        let value = if self.peek() == Some(b'"') {
            self.quoted()
        } else {
            self.token().to_owned()
        };
        Some((name.to_ascii_lowercase(), value))
    }

    fn token(&mut self) -> &'a str {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| !matches!(b, b';' | b',' | b'=' | b' ' | b'\t' | b'"'))
        {
            self.pos += 1;
        }
        &self.src[start..self.pos]
    }

    fn quoted(&mut self) -> String {
        self.pos += 1;
        let mut out = String::new();
        let mut chars = self.src[self.pos..].char_indices();
        while let Some((offset, c)) = chars.next() {
            match c {
                '"' => {
                    self.pos += offset + 1;
                    return out;
                }
                '\\' => out.extend(chars.next().map(|(_, escaped)| escaped)),
                other => out.push(other),
            }
        }
        self.pos = self.src.len();
        out
    }

    fn skip_value(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b',' => return,
                b'"' => {
                    self.quoted();
                }
                b'<' => match self.src[self.pos..].find('>') {
                    Some(end) => self.pos += end + 1,
                    None => self.pos = self.src.len(),
                },
                _ => self.pos += 1,
            }
        }
    }
}
