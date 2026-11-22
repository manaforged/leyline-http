use std::time::{Duration, SystemTime};

use super::compute_expiry;

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
