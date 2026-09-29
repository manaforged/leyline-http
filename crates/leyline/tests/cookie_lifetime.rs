#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use leyline::cookie::{Cookie, Jar};
use url::Url;

const LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);
const SLACK: Duration = Duration::from_secs(300);
const TEN_YEARS: Duration = Duration::from_secs(10 * 365 * 24 * 60 * 60);

fn assert_capped(expires: Option<SystemTime>) {
    let cap = SystemTime::now() + LIFETIME;
    let expires = expires.unwrap();
    assert!(expires <= cap, "{expires:?} is past {cap:?}");
    assert!(expires + SLACK >= cap, "{expires:?} is short of {cap:?}");
}

fn stored(set_cookie: &str) -> Cookie {
    let jar = Jar::new();
    jar.store_set_cookie(set_cookie, &Url::parse("https://example.com/").unwrap());
    jar.all_cookies().pop().unwrap()
}

fn loaded(expires_ms: i64) -> Cookie {
    let mut value = serde_json::to_value(stored("a=b")).unwrap();
    value["expires"] = serde_json::json!(expires_ms);
    serde_json::from_value(value).unwrap()
}

#[test]
fn an_expires_date_past_the_platform_clock_is_capped() {
    assert_capped(stored("a=b; Expires=Fri, 01 Jan 99999 00:00:00 GMT").expires);
}

#[test]
fn a_loaded_expiry_past_400_days_is_capped() {
    let far = SystemTime::now() + TEN_YEARS;
    let ms = i64::try_from(far.duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
    assert_capped(loaded(ms).expires);
}

#[test]
fn a_loaded_expiry_past_the_platform_clock_is_capped() {
    assert_capped(loaded(i64::MAX).expires);
}
