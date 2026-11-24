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
