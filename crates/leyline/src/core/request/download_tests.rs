use super::range;
use crate::util::every_input;

const PREFIXES: [&str; 4] = ["bytes ", "bytes", "Bytes ", ""];
const TAIL: [&[u8]; 6] = [b"0", b"1", b"9", b"-", b"/", b" "];

fn number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())?
}

fn rfc9110(header: &str, offset: u64) -> Option<u64> {
    let (span, complete) = header.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    let (first, last, complete) = (number(first)?, number(last)?, number(complete)?);
    (first == offset && first <= last && last + 1 == complete).then_some(complete)
}

#[test]
fn every_short_content_range_matches_the_complete_remainder() {
    for prefix in PREFIXES {
        every_input(&TAIL, 6, |tail| {
            let header = format!("{prefix}{}", std::str::from_utf8(tail).expect("ASCII"));
            for offset in [0, 1, 9, 10] {
                assert_eq!(
                    range(&header, offset),
                    rfc9110(&header, offset),
                    "{header:?} at {offset}"
                );
            }
        });
    }
}
