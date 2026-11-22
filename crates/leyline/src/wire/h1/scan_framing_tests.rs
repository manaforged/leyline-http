use super::super::BodyFraming;
use super::scan_head;

const LENGTHS: [&str; 10] = [
    "0",
    "5",
    "05",
    "+5",
    "-1",
    "5,5",
    "5 5",
    "a",
    "18446744073709551615",
    "18446744073709551616",
];
const CODINGS: [&str; 9] = [
    "chunked",
    "CHUNKED",
    "gzip",
    "gzip, chunked",
    "chunked, gzip",
    "chunked, chunked",
    "identity",
    "",
    ",",
];

#[derive(Debug, PartialEq)]
enum Framed {
    Chunked,
    Fixed(u64),
    ToClose,
}

fn rfc_9112_section_6(lengths: &[&str], codings: &[&str]) -> Option<Framed> {
    let combined = codings.join(",");
    let tokens: Vec<String> = combined
        .split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if !codings.is_empty() {
        let chunked = tokens.iter().filter(|t| *t == "chunked").count();
        let last_is_chunked = tokens.last().is_some_and(|t| t == "chunked");
        return (lengths.is_empty() && last_is_chunked && chunked == 1).then_some(Framed::Chunked);
    }
    match lengths {
        [] => Some(Framed::ToClose),
        [one] if one.bytes().all(|b| b.is_ascii_digit()) => one.parse().ok().map(Framed::Fixed),
        _ => None,
    }
}

fn leyline(lengths: &[&str], codings: &[&str]) -> Option<Framed> {
    let mut head = String::from("HTTP/1.1 200 OK\r\n");
    for value in lengths {
        head += &format!("content-length: {value}\r\n");
    }
    for value in codings {
        head += &format!("transfer-encoding: {value}\r\n");
    }
    head += "\r\n";
    let (_, _, scanned) = scan_head(head.as_bytes()).ok()?;
    scanned.body.validate().ok()?;
    Some(match scanned.body.framing() {
        BodyFraming::Chunked => Framed::Chunked,
        BodyFraming::Fixed(length) => Framed::Fixed(length),
        BodyFraming::ToClose | BodyFraming::None => Framed::ToClose,
    })
}

fn up_to_two<'a>(values: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut all = vec![Vec::new()];
    for first in values {
        all.push(vec![*first]);
        for second in values {
            all.push(vec![*first, *second]);
        }
    }
    all
}

#[test]
fn every_framing_header_set_follows_rfc_9112_section_6() {
    let mut wrong = Vec::new();
    for lengths in up_to_two(&LENGTHS) {
        for codings in up_to_two(&CODINGS) {
            let (got, want) = (
                leyline(&lengths, &codings),
                rfc_9112_section_6(&lengths, &codings),
            );
            if got != want {
                wrong.push(format!(
                    "cl={lengths:?} te={codings:?} got={got:?} want={want:?}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} disagreements, first: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(25)]
    );
}

#[test]
fn valid_framing_matches_check_framing() {
    use std::borrow::Cow;

    use super::BodyHeaders;

    let to_close = BodyHeaders {
        lengths: 0,
        length: None,
        coding: None,
    };
    assert!(to_close.check_framing().is_ok());
    assert!(matches!(to_close.framing(), BodyFraming::ToClose));

    let fixed_zero = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"0")),
        coding: None,
    };
    assert!(fixed_zero.check_framing().is_ok());
    assert!(matches!(fixed_zero.framing(), BodyFraming::Fixed(0)));

    let fixed_val = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"42")),
        coding: None,
    };
    assert!(fixed_val.check_framing().is_ok());
    assert!(matches!(fixed_val.framing(), BodyFraming::Fixed(42)));

    let chunked = BodyHeaders {
        lengths: 0,
        length: None,
        coding: Some(Cow::Borrowed(b"chunked")),
    };
    assert!(chunked.check_framing().is_ok());
    assert!(matches!(chunked.framing(), BodyFraming::Chunked));

    let bad = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"bad")),
        coding: None,
    };
    assert!(bad.check_framing().is_err());
}
