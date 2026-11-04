use std::time::Duration;

use bytes::Bytes;

use super::Parser;
use crate::util::every_input;

const TOKENS: [&[u8]; 9] = [
    b"data:", b"data", b":", b" ", b"x", b"event:e", b"\n", b"\r", b"\r\n",
];

fn whatwg(input: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(input)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.pop();
    let (mut events, mut data, mut event) = (Vec::new(), String::new(), String::new());
    for line in lines {
        if line.is_empty() {
            let name = std::mem::take(&mut event);
            if let Some(body) = std::mem::take(&mut data).strip_suffix('\n') {
                events.push((
                    if name.is_empty() {
                        "message".into()
                    } else {
                        name
                    },
                    body.to_owned(),
                ));
            }
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => data = format!("{data}{value}\n"),
            "event" => event = value.to_owned(),
            _ => {}
        }
    }
    events
}

fn parse(input: &[u8], split: usize, max: usize) -> Result<Vec<(String, String)>, ()> {
    let mut parser = Parser::new(String::new(), max, Duration::ZERO);
    let mut events = Vec::new();
    for chunk in [&input[..split], &input[split..]] {
        let mut bytes = Bytes::copy_from_slice(chunk);
        loop {
            let read = parser.read(&mut bytes).map_err(drop)?;
            assert!(parser.buffered() <= max, "buffered past max {max}");
            match read {
                Some(event) => {
                    assert!(event.data.len() <= max, "event past max {max}");
                    events.push((event.event, event.data));
                }
                None => break,
            }
        }
    }
    Ok(events)
}

#[test]
fn every_short_event_stream_matches_the_whatwg_model_at_every_split() {
    every_input(&TOKENS, 5, |input| {
        for split in 0..=input.len() {
            assert_eq!(
                parse(input, split, 1 << 20),
                Ok(whatwg(input)),
                "{:?} split {split}",
                String::from_utf8_lossy(input)
            );
        }
    });
}

#[test]
fn every_short_event_stream_stays_within_max_event_size() {
    every_input(&TOKENS, 5, |input| {
        for max in 0..4 {
            drop(parse(input, input.len() / 2, max));
        }
    });
}
