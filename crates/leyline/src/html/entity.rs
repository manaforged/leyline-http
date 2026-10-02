include!(concat!(env!("OUT_DIR"), "/entities.rs"));

const MAX_NAME: usize = 32;

const C1_FIRST: u32 = 0x80;

const C1: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Text,
    Attribute,
}

pub(super) fn decode(text: &str) -> String {
    decode_in(text, Context::Text)
}

pub(super) fn decode_attribute(text: &str) -> String {
    decode_in(text, Context::Attribute)
}

fn decode_in(text: &str, context: Context) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let used = match rest.strip_prefix('#') {
            Some(number) => numeric(number, &mut out).map(|len| len + 1),
            None => named(rest, context, &mut out),
        };
        match used {
            Some(len) => rest = &rest[len..],
            None => out.push('&'),
        }
    }
    out.push_str(rest);
    out
}

fn named(text: &str, context: Context, out: &mut String) -> Option<usize> {
    let run = text
        .bytes()
        .take(MAX_NAME)
        .take_while(u8::is_ascii_alphanumeric)
        .count();
    if text[run..].starts_with(';')
        && let Some(value) = lookup(&text[..=run])
    {
        out.push_str(value);
        return Some(run + 1);
    }
    let (len, value) = (1..=run)
        .rev()
        .find_map(|len| lookup(&text[..len]).map(|value| (len, value)))?;
    let next = text.as_bytes().get(len);
    if context == Context::Attribute
        && next.is_some_and(|b| *b == b'=' || b.is_ascii_alphanumeric())
    {
        return None;
    }
    out.push_str(value);
    Some(len)
}

fn lookup(name: &str) -> Option<&'static str> {
    ENTITIES
        .binary_search_by(|(key, _)| key.cmp(&name))
        .ok()
        .map(|at| ENTITIES[at].1)
}

fn numeric(text: &str, out: &mut String) -> Option<usize> {
    let (radix, start) = match text.as_bytes().first() {
        Some(b'x' | b'X') => (16, 1),
        _ => (10, 0),
    };
    let digits: Vec<u32> = text[start..]
        .bytes()
        .map_while(|b| char::from(b).to_digit(radix))
        .collect();
    if digits.is_empty() {
        return None;
    }
    let code = digits.iter().fold(0u32, |code, digit| {
        code.saturating_mul(radix).saturating_add(*digit)
    });
    out.push(resolve(code));
    let len = start + digits.len();
    Some(len + usize::from(text[len..].starts_with(';')))
}

fn resolve(code: u32) -> char {
    let code = code
        .checked_sub(C1_FIRST)
        .and_then(|offset| C1.get(usize::try_from(offset).ok()?))
        .copied()
        .unwrap_or(code);
    match code {
        0 => char::REPLACEMENT_CHARACTER,
        code => char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER),
    }
}
