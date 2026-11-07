use super::*;

#[test]
fn base64_encode_works() {
    assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
    assert_eq!(base64_encode("a"), "YQ==");
    assert_eq!(base64_encode("ab"), "YWI=");
}

#[test]
fn percent_decode_roundtrips_common_cases() {
    assert_eq!(percent_decode("a%3Db"), "a=b");
    assert_eq!(percent_decode("plain"), "plain");
    assert_eq!(percent_decode("s3cr3t%21"), "s3cr3t!");
    assert_eq!(percent_decode("bad%"), "bad%");
}

#[test]
fn idempotent_matches_rfc_set() {
    for m in ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"] {
        assert!(is_idempotent(m), "{m}");
        assert!(is_idempotent(&m.to_lowercase()), "{m}");
    }
    for m in ["POST", "PATCH"] {
        assert!(!is_idempotent(m), "{m}");
    }
}

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
