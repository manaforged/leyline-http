use std::path::PathBuf;
use std::time::Duration;

use leyline::cookie::Jar;
use leyline::testing::{self, queue};
use leyline::{Browser, Session, SessionBuilder};
use serde::{Deserialize, Serialize};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-state-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn saved_cookie(path: &std::path::Path) -> Option<String> {
    let url = "https://shop.example/".parse().unwrap();
    Session::load(path)
        .unwrap()
        .build()
        .unwrap()
        .cookies()
        .get(&url, "sid")
}

#[test]
fn a_saved_session_names_its_format_version() {
    let dir = scratch("version");
    let path = dir.join("device.json");
    let session = Session::new();
    session
        .cookies()
        .set(&"https://shop.example/".parse().unwrap(), "sid", "1");
    session.save(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["jar"].as_array().map(Vec::len), Some(1));
    assert_eq!(saved_cookie(&path).as_deref(), Some("1"));
    drop(std::fs::remove_dir_all(&dir));
}

#[derive(Serialize)]
struct AccountOut<'a> {
    session: &'a Session,
    email: String,
}

#[derive(Deserialize)]
struct AccountIn {
    session: SessionBuilder,
    email: String,
}

#[test]
fn a_user_struct_saves_its_own_fields_beside_the_session() {
    let url = "https://shop.example/".parse().unwrap();
    let session = Session::from(Browser::Chrome154);
    session.cookies().set(&url, "sid", "1");
    let text = serde_json::to_string(&AccountOut {
        session: &session,
        email: "a@shop.example".to_owned(),
    })
    .unwrap();
    let account: AccountIn = serde_json::from_str(&text).unwrap();
    assert_eq!(account.email, "a@shop.example");
    let reopened = account.session.build().unwrap();
    assert_eq!(reopened.cookies().get(&url, "sid").as_deref(), Some("1"));
    assert_eq!(reopened.profile_id(), session.profile_id());
}

fn autosave_keeps_a_change_on(runtime: tokio::runtime::Runtime, name: &str) {
    let dir = scratch(name);
    let path = dir.join("device.json");
    let session = Session::new();
    runtime.block_on(async { session.autosave(&path, Duration::from_secs(60)) });
    session
        .cookies()
        .set(&"https://shop.example/".parse().unwrap(), "sid", "1");
    drop(runtime);
    assert_eq!(saved_cookie(&path).as_deref(), Some("1"));
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn autosave_writes_the_last_change_when_the_runtime_stops() {
    autosave_keeps_a_change_on(tokio::runtime::Runtime::new().unwrap(), "runtime");
}

#[test]
fn autosave_writes_a_change_made_before_its_task_first_runs() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    autosave_keeps_a_change_on(runtime, "unpolled");
}

#[test]
fn autosave_after_shutdown_and_runtime_drop_keeps_the_last_change() {
    let dir = scratch("aftershutdown");
    let path = dir.join("device.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let session = Session::new();
    runtime.block_on(async { session.autosave(&path, Duration::from_secs(60)) });
    session
        .cookies()
        .set(&"https://shop.example/".parse().unwrap(), "sid", "1");
    session.shutdown();
    drop(runtime);
    assert_eq!(saved_cookie(&path).as_deref(), Some("1"));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn one_http1_origin_counts_one_install() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .build()
        .unwrap();
    session.get(server.url("/")).send().await.unwrap();
    assert_eq!(session.stats().installs(), 1);
}

#[tokio::test]
async fn audit_flags_a_header_the_echo_did_not_see() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let resp = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .send()
        .await
        .unwrap();
    let audit = resp.fingerprint().unwrap();
    let agent = audit
        .headers()
        .iter()
        .find(|(name, _)| name == "user-agent")
        .map(|(_, value)| value.clone())
        .unwrap();
    let echoed = leyline::Fingerprint::from_json(&format!(
        r#"{{"http1":{{"headers":["User-Agent: {agent}"]}}}}"#
    ))
    .unwrap();
    let diff = audit.diff(&echoed);
    assert!(
        diff.iter().any(|(field, _, _)| field == "accept"),
        "{diff:?}"
    );
}

#[test]
fn autosave_writes_new_session_state_when_the_runtime_stops() {
    let dir = scratch("devstate");
    let path = dir.join("device.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime
        .block_on(testing::Server::https(queue([
            testing::Response::new(200).header("strict-transport-security", "max-age=3600")
        ])))
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .build()
        .unwrap();
    runtime.block_on(async {
        session.autosave(&path, Duration::from_secs(3600));
        session
            .get(format!("https://localhost:{}/", server.addr().port()))
            .send()
            .await
            .unwrap();
    });
    drop(runtime);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let state = saved["state"].to_string();
    assert!(state.contains("localhost"), "{state}");
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn serializing_a_session_never_writes_its_proxy_password() {
    let session = Session::builder()
        .browser(Browser::default())
        .proxy("http://user:hunter2@proxy.example:8080")
        .build()
        .unwrap();
    let text = serde_json::to_string(&session).unwrap();
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("proxy.example"), "{text}");
}

#[tokio::test]
async fn audit_matches_a_cookie_split_across_fields() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let jar = Jar::new();
    let url = server.url("/").parse().unwrap();
    jar.set(&url, "a", "1");
    jar.set(&url, "b", "2");
    let resp = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .cookies(jar)
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .send()
        .await
        .unwrap();
    let audit = resp.fingerprint().unwrap();
    let sent = audit
        .headers()
        .iter()
        .find(|(name, _)| name == "cookie")
        .map(|(_, value)| value.clone())
        .unwrap();
    let lines: Vec<String> = sent
        .split("; ")
        .map(|pair| format!("\"Cookie: {pair}\""))
        .collect();
    let echoed = leyline::Fingerprint::from_json(&format!(
        r#"{{"http1":{{"headers":[{}]}}}}"#,
        lines.join(",")
    ))
    .unwrap();
    let diff = audit.diff(&echoed);
    assert!(
        !diff.iter().any(|(field, _, _)| field == "cookie"),
        "{diff:?}"
    );
}
