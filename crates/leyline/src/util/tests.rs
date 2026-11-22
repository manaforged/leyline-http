use super::*;

#[test]
fn lock_recovers_after_panic() {
    let cache = std::sync::Arc::new(std::sync::Mutex::new(vec![1u8]));
    let poisoner = cache.clone();
    drop(
        std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the mutex");
        })
        .join(),
    );
    assert!(cache.is_poisoned(), "test setup failed to poison the mutex");

    lock(&cache).push(2);
    assert_eq!(*lock(&cache), vec![1, 2]);
}

#[test]
fn the_word_uppercase_check_matches_the_byte_check_at_every_length_and_position() {
    let probes = [
        '@', 'A', 'M', 'Z', '[', '`', 'a', 'z', '{', '\u{7f}', 'é', 'Ä', '€',
    ];
    for len in 0..=24 {
        let plain = "x".repeat(len);
        assert!(!has_ascii_uppercase(&plain), "{plain:?}");
        for at in 0..len {
            for probe in probes {
                let text: String = plain
                    .chars()
                    .enumerate()
                    .map(|(i, c)| if i == at { probe } else { c })
                    .collect();
                let want = text.bytes().any(|b| b.is_ascii_uppercase());
                assert_eq!(has_ascii_uppercase(&text), want, "{text:?}");
            }
        }
    }
    for byte in 0u8..=0x7f {
        for len in [8, 9, 16, 17] {
            let mut text = vec![b'x'; len];
            text[len - 1] = byte;
            let text = String::from_utf8(text).expect("ascii");
            assert_eq!(
                has_ascii_uppercase(&text),
                byte.is_ascii_uppercase(),
                "{text:?}"
            );
        }
    }
}

#[test]
fn parse_ascii_u64_and_delta_seconds_match_spec_exhaustively() {
    let alphabet: &[&[u8]] = &[b"0", b"9", b" ", b"\t", b"+", b"-", b"a"];
    every_input(alphabet, 4, |slice| {
        let s = std::str::from_utf8(slice).expect("ascii");
        let all_digits = !slice.is_empty() && slice.iter().all(u8::is_ascii_digit);
        assert_eq!(parse_ascii_u64(s).is_some(), all_digits, "{s:?}");
        let trimmed = s.trim();
        let trimmed_digits = !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit());
        assert_eq!(delta_seconds(s).is_some(), trimmed_digits, "{s:?}");
        if all_digits {
            assert_eq!(parse_ascii_u64(s), delta_seconds(s), "{s:?}");
        }
    });
    assert_eq!(parse_ascii_u64("18446744073709551615"), Some(u64::MAX));
    assert_eq!(parse_ascii_u64("18446744073709551616"), None);
    assert_eq!(delta_seconds("18446744073709551616"), Some(u64::MAX));
}

#[test]
fn bare_host_strips_only_matched_brackets_exhaustively() {
    let alphabet: &[&[u8]] = &[b"[", b"]", b":", b"a", b"."];
    every_input(alphabet, 6, |slice| {
        let s = std::str::from_utf8(slice).expect("ascii");
        let out = bare_host(s);
        if slice.len() >= 2 && slice[0] == b'[' && slice[slice.len() - 1] == b']' {
            assert_eq!(out.as_bytes(), &slice[1..slice.len() - 1], "{s:?}");
        } else {
            assert_eq!(out, s, "{s:?}");
        }
    });
}

#[test]
fn domain_match_enforces_label_boundary_exhaustively() {
    let alphabet: &[&[u8]] = &[b".", b"a", b"A", b"b"];
    let mut domains = Vec::new();
    every_input(alphabet, 3, |slice| {
        if !slice.is_empty() {
            domains.push(std::str::from_utf8(slice).expect("ascii").to_owned());
        }
    });
    every_input(alphabet, 5, |host_slice| {
        if host_slice.is_empty() {
            return;
        }
        let host = std::str::from_utf8(host_slice).expect("ascii");
        for dom in &domains {
            let dom_slice = dom.as_bytes();
            let matched = domain_match(host, dom);
            let expected = if host_slice.len() < dom_slice.len() {
                false
            } else if host_slice.len() == dom_slice.len() {
                host_slice.eq_ignore_ascii_case(dom_slice)
            } else {
                let rest = host_slice.len() - dom_slice.len();
                host_slice[rest - 1] == b'.' && host_slice[rest..].eq_ignore_ascii_case(dom_slice)
            };
            assert_eq!(matched, expected, "host={host:?} dom={dom:?}");
        }
    });
}
