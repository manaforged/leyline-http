use url::Url;

const WILDCARD: char = '*';
const END: char = '$';

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    Byte(u8),
    Any,
}

#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    tokens: Vec<Token>,
    anchored: bool,
}

impl Pattern {
    pub(crate) fn parse(raw: &str) -> Option<Pattern> {
        if raw.is_empty() {
            return None;
        }
        let (body, anchored) = match raw.strip_suffix(END) {
            Some(body) => (body, true),
            None => (raw, false),
        };
        let mut tokens = Vec::new();
        for (index, piece) in body.split(WILDCARD).enumerate() {
            if index > 0 {
                tokens.push(Token::Any);
            }
            let mut bytes = Vec::new();
            encode(piece, &mut bytes, true);
            tokens.extend(bytes.into_iter().map(Token::Byte));
        }
        Some(Pattern { tokens, anchored })
    }

    pub(super) fn specificity(&self) -> usize {
        self.tokens.len() + usize::from(self.anchored)
    }

    pub(crate) fn matches(&self, path: &[u8]) -> bool {
        let (mut at, mut read) = (0, 0);
        let mut retry: Option<(usize, usize)> = None;
        loop {
            match self.tokens.get(at) {
                None if !self.anchored || read == path.len() => return true,
                Some(Token::Any) => {
                    retry = Some((at, read));
                    at += 1;
                    continue;
                }
                Some(Token::Byte(byte)) if path.get(read) == Some(byte) => {
                    at += 1;
                    read += 1;
                    continue;
                }
                _ => {}
            }
            match retry {
                Some((star, from)) if from < path.len() => {
                    retry = Some((star, from + 1));
                    at = star + 1;
                    read = from + 1;
                }
                _ => return false,
            }
        }
    }
}

pub(crate) fn path_bytes(url: &Url) -> Vec<u8> {
    let mut raw = url.path().to_owned();
    if let Some(query) = url.query() {
        raw.push('?');
        raw.push_str(query);
    }
    let mut bytes = Vec::with_capacity(raw.len());
    encode(&raw, &mut bytes, true);
    bytes
}

#[cfg(feature = "html")]
pub(crate) fn normalize(text: &str) -> String {
    let mut bytes = Vec::with_capacity(text.len());
    encode(text, &mut bytes, false);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn encode(text: &str, out: &mut Vec<u8>, escape_specials: bool) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%'
            && let Some(value) = bytes.get(at + 1..at + 3).and_then(hex_pair)
        {
            if unreserved(value) {
                out.push(value);
            } else {
                escape(out, value);
            }
            at += 3;
            continue;
        }
        let special = escape_specials && matches!(byte, b'*' | b'$');
        if special || !byte.is_ascii_graphic() {
            escape(out, byte);
        } else {
            out.push(byte);
        }
        at += 1;
    }
}

fn hex_pair(pair: &[u8]) -> Option<u8> {
    if !pair.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let text = std::str::from_utf8(pair).ok()?;
    u8::from_str_radix(text, 16).ok()
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn escape(out: &mut Vec<u8>, byte: u8) {
    out.extend_from_slice(format!("%{byte:02X}").as_bytes());
}
