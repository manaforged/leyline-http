const NAMED: [(&str, char); 6] = [
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
];

const MAX_ENTITY: usize = 32;

pub(super) fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match entity(rest) {
            Some((decoded, len)) => {
                out.push(decoded);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(text: &str) -> Option<(char, usize)> {
    let end = text.bytes().take(MAX_ENTITY).position(|b| b == b';')?;
    let body = &text[1..end];
    let decoded = match body.strip_prefix('#') {
        Some(number) => numeric(number)?,
        None => NAMED.iter().find(|(name, _)| *name == body)?.1,
    };
    Some((decoded, end + 1))
}

fn numeric(number: &str) -> Option<char> {
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).ok()?
        }
        Some(_) => return None,
        None if number.bytes().all(|b| b.is_ascii_digit()) => number.parse().ok()?,
        None => return None,
    };
    Some(match code {
        0 => char::REPLACEMENT_CHARACTER,
        code => char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER),
    })
}
