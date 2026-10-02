use std::time::{Duration, SystemTime, UNIX_EPOCH};

use url::Url;

use crate::cookie::Jar;
use crate::util::lock;

fn url() -> Url {
    Url::parse("https://www.example.com/").unwrap()
}

fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn saved(name: &str, value: &str, expires: Option<i64>, created: i64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "value": value,
        "domain": "www.example.com",
        "path": "/",
        "secure": true,
        "http_only": false,
        "same_site": "Lax",
        "expires": expires,
        "creation_time": created,
        "host_only": true,
    })
}

#[test]
fn changes_fire_on_writes_through_any_clone_and_not_on_reads() {
    let jar = Jar::new();
    let mut changes = jar.changes();

    jar.clone()
        .store_set_cookie("session-token=a; Path=/", &url());
    assert!(changes.has_changed().unwrap());
    changes.mark_unchanged();

    assert!(jar.cookie_header(&url()).is_some());
    assert!(!changes.has_changed().unwrap());

    jar.remove_named("session-token");
    assert!(changes.has_changed().unwrap());
}

#[test]
fn load_drops_expired_cookies_and_keeps_the_newest_duplicate() {
    let now = millis(SystemTime::now());
    let saved = serde_json::Value::Array(vec![
        saved("gone", "1", Some(now - 60_000), now - 120_000),
        saved("token", "old", None, now - 60_000),
        saved("token", "new", None, now - 1_000),
    ]);
    let jar: Jar = serde_json::from_value(saved).unwrap();
    let cookies = jar.all_cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "token");
    assert_eq!(cookies[0].value, "new");
}

#[test]
fn save_drops_cookies_that_expired_in_memory() {
    let jar = Jar::new();
    jar.store_set_cookie("brief=1; Path=/; Max-Age=3600", &url());
    jar.store_set_cookie("kept=1; Path=/; Max-Age=3600", &url());
    for cookie in lock(&jar.inner).cookies.values_mut().flatten() {
        if cookie.name == "brief" {
            cookie.expires = Some(SystemTime::now() - Duration::from_secs(1));
        }
    }
    let saved = serde_json::to_value(&jar).unwrap();
    let names: Vec<&str> = saved
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["kept"]);
}

#[test]
fn last_access_survives_a_round_trip() {
    let jar = Jar::new();
    jar.store_set_cookie("token=a; Path=/", &url());
    jar.cookie_header(&url());
    let accessed = millis(jar.all_cookies()[0].last_access);

    let restored: Jar = serde_json::from_str(&serde_json::to_string(&jar).unwrap()).unwrap();
    assert_eq!(millis(restored.all_cookies()[0].last_access), accessed);
}

#[test]
fn jar_saved_without_last_access_still_loads() {
    let now = millis(SystemTime::now());
    let jar: Jar = serde_json::from_value(serde_json::Value::Array(vec![saved(
        "token", "a", None, now,
    )]))
    .unwrap();
    assert_eq!(jar.get_cookie(&url(), "token").as_deref(), Some("a"));
}
