use std::time::{Duration, SystemTime};

use super::{CookieAttributes, compute_expiry, prefix_rejected};

#[kani::proof]
fn zero_max_age_always_expires_at_unix_epoch() {
    let offset: u64 = kani::any_where(|&s| s <= 3_000_000_000);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(offset);
    let fallback: Option<Duration> = if kani::any() {
        Some(Duration::from_secs(kani::any_where(|&s| {
            s <= 3_000_000_000
        })))
    } else {
        None
    };
    assert_eq!(
        compute_expiry(now, Some(Duration::ZERO), fallback),
        Some(SystemTime::UNIX_EPOCH)
    );
}

#[kani::proof]
#[kani::unwind(12)]
fn cookie_prefix_rules_enforce_rfc_6265bis() {
    const NAMES: [&str; 4] = ["__Secure-a", "__SECURE-b", "__Host-c", "__host-d"];
    let idx: usize = kani::any_where(|&i| i < 4);
    let secure: bool = kani::any();
    let has_domain: bool = kani::any();
    let path_kind: u8 = kani::any_where(|&k| k < 3);
    let attrs = CookieAttributes {
        domain: has_domain.then(|| String::from("example.com")),
        path: match path_kind {
            0 => None,
            1 => Some(String::from("/")),
            _ => Some(String::from("/app")),
        },
        secure,
        ..CookieAttributes::default()
    };
    let rejected = prefix_rejected(NAMES[idx], &attrs);
    if idx < 2 {
        assert_eq!(rejected, !secure);
    } else {
        assert_eq!(rejected, !secure || has_domain || path_kind != 1);
    }
}

#[kani::proof]
#[kani::unwind(6)]
fn domain_match_enforces_label_boundary() {
    let host_buf: [u8; 5] = kani::any();
    let dom_buf: [u8; 3] = kani::any();
    let host_len: usize = kani::any_where(|&n| (1..=5).contains(&n));
    let dom_len: usize = kani::any_where(|&n| (1..=3).contains(&n));
    let host_slice = &host_buf[..host_len];
    let dom_slice = &dom_buf[..dom_len];
    kani::assume(host_slice.iter().all(|b| b.is_ascii()));
    kani::assume(dom_slice.iter().all(|b| b.is_ascii()));
    let host = std::str::from_utf8(host_slice).expect("ascii");
    let dom = std::str::from_utf8(dom_slice).expect("ascii");
    let matched = crate::util::domain_match(host, dom);
    if host_len < dom_len {
        assert!(!matched);
    } else if host_len == dom_len {
        assert_eq!(matched, host_slice.eq_ignore_ascii_case(dom_slice));
    } else {
        let rest = host_len - dom_len;
        let expected =
            host_slice[rest - 1] == b'.' && host_slice[rest..].eq_ignore_ascii_case(dom_slice);
        assert_eq!(matched, expected);
    }
}
