use std::path::PathBuf;
use std::time::Duration;

use leyline::audit::Observed;
use leyline::cookie::Jar;
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, Device, DeviceAutosaveOptions, Session};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("leyline-state-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_saved_jar_names_its_format_version() {
    let dir = scratch("version");
    let path = dir.join("jar.json");
    let jar = Jar::new();
    jar.set_cookie(&"https://shop.example/".parse().unwrap(), "sid", "1");
    jar.save_to(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["cookies"].as_array().map(Vec::len), Some(1));
    drop(std::fs::remove_dir_all(&dir));
}

#[test]
fn autosave_writes_the_last_change_when_the_runtime_stops() {
    let dir = scratch("runtime");
    let path = dir.join("jar.json");
    let url = "https://shop.example/".parse().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let jar = Jar::new();
    let handle = runtime.block_on(async { jar.autosave(&path, Duration::from_secs(60)) });
    jar.set_cookie(&url, "sid", "1");
    drop(runtime);
    drop(handle);
    let loaded = Jar::load_from(&path).unwrap();
    assert_eq!(loaded.get_cookie(&url, "sid").as_deref(), Some("1"));
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn one_http1_origin_counts_one_install() {
    let server = TestServer::https(queue([TestResponse::new(200)]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .build()
        .unwrap();
    session.get(server.url("/")).await.unwrap();
    assert_eq!(session.pool_stats().installs, 1);
}

#[tokio::test]
async fn audit_flags_a_header_the_echo_did_not_see() {
    let server = TestServer::https(queue([TestResponse::new(200)]))
        .await
        .unwrap();
    let resp = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .await
        .unwrap();
    let audit = resp.audit().unwrap();
    let agent = audit
        .request_headers
        .iter()
        .find(|(name, _)| name == "user-agent")
        .map(|(_, value)| value.clone())
        .unwrap();
    let echoed = Observed::from_json(&format!(
        r#"{{"http1":{{"headers":["User-Agent: {agent}"]}}}}"#
    ))
    .unwrap();
    let report = audit.compare(&echoed);
    let accept = report
        .headers
        .iter()
        .find(|h| h.name == "accept")
        .map(|h| h.outcome.clone())
        .unwrap();
    assert!(accept.is_mismatch(), "{accept:?}");
    assert!(!report.is_match());
}

#[test]
fn device_autosave_writes_new_session_state_when_the_runtime_stops() {
    let dir = scratch("devstate");
    let path = dir.join("device.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime
        .block_on(TestServer::https(queue([
            TestResponse::new(200).header("strict-transport-security", "max-age=3600")
        ])))
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .build()
        .unwrap();
    let device = Device::capture(&session, None);
    let options = DeviceAutosaveOptions::new(Duration::from_secs(60))
        .state_interval(Duration::from_secs(3600));
    let handle = runtime.block_on(async {
        let handle = device.autosave(&session, &path, options);
        handle.flush().await.unwrap();
        session
            .get(format!("https://localhost:{}/", server.addr().port()))
            .await
            .unwrap();
        handle
    });
    drop(runtime);
    drop(handle);
    let saved = Device::load_from(&path).unwrap();
    let state = serde_json::to_string(&saved.state).unwrap();
    assert!(state.contains("localhost"), "{state}");
    drop(std::fs::remove_dir_all(&dir));
}
