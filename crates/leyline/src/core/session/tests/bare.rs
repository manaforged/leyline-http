//! Bare (non-impersonating) default session behaviour.
//!
//! The default `Session` does NOT impersonate a browser: a plain
//! `leyline/<version>` User-Agent, no `sec-ch-ua` / client-hint headers,
//! and the host OS rather than a hardcoded Windows fingerprint. Opting into
//! a browser is the explicit, named action.

use crate::profile::{Browser, Platform};
use crate::Session;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn capture_get_headers(session: Session) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8_lossy(&req).to_string()
    });
    let _ = session
        .get(&format!("http://{addr}/"))
        .send()
        .await
        .unwrap();
    server.await.unwrap()
}

#[test]
fn default_session_is_bare() {
    // No `.browser(...)` → bare. `Session::new()` and a plain builder
    // both land here.
    assert_eq!(Session::new().browser(), None);
    assert_eq!(Session::builder().build().unwrap().browser(), None);
}

#[test]
fn chrome_helper_is_explicit_browser() {
    // Impersonation is the named opt-in, and still selects a real browser.
    assert_eq!(
        Session::chrome().browser(),
        Some(Browser::default_browser())
    );
}

#[test]
fn bare_default_platform_follows_host() {
    // A bare session with no explicit `.platform()` resolves to the host
    // OS, not a hardcoded Windows.
    let s = Session::new();
    assert_eq!(s.platform(), Platform::detect_host());
    // An impersonation profile, by contrast, defaults to Windows.
    let chrome = Session::builder()
        .browser(Browser::Chrome148)
        .build()
        .unwrap();
    assert_eq!(chrome.platform(), Platform::Windows);
}

#[tokio::test]
async fn bare_session_sends_generic_ua_and_no_client_hints() {
    let req = capture_get_headers(Session::new()).await;
    let lower = req.to_lowercase();
    assert!(
        lower.contains("user-agent: leyline/"),
        "bare UA should be leyline/<version>:\n{req}"
    );
    assert!(
        !lower.contains("sec-ch-ua"),
        "bare session must not emit client-hint headers:\n{req}"
    );
    assert!(
        !lower.contains("chrome") && !lower.contains("mozilla"),
        "bare session must not look like a browser:\n{req}"
    );
}

#[test]
fn platform_host_resolves_and_never_leaks() {
    assert_eq!(Platform::Host.resolve(), Platform::detect_host());
    assert_ne!(Platform::detect_host(), Platform::Host);
    // Explicit platform still wins and is honoured verbatim.
    let s = Session::builder()
        .browser(Browser::Chrome148)
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    assert_eq!(s.platform(), Platform::MacOS);
}
