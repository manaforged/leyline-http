use leyline::{Browser, Platform};

use super::tls_peet::peet;

async fn observe_ttl(browser: Browser, platform: Platform) -> i64 {
    let session = leyline::Session::builder()
        .browser(browser)
        .platform(platform)
        .build()
        .unwrap();
    let json = peet(&session).await;
    json["tcpip"]["ip"]["ttl"]
        .as_i64()
        .expect("no tcpip.ip.ttl")
}

fn initial_ttl(observed: i64) -> i64 {
    [64, 128, 255]
        .into_iter()
        .find(|&t| observed <= t)
        .unwrap_or(255)
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_ttl_is_128() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    assert_eq!(
        initial_ttl(ttl),
        128,
        "Windows initial TTL should be 128, observed {ttl} (rounded to {})",
        initial_ttl(ttl)
    );
    println!("✓ Windows TCP TTL: observed {ttl} (initial 128)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_linux_ttl_is_64() {
    let ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    assert_eq!(
        initial_ttl(ttl),
        64,
        "Linux initial TTL should be 64, observed {ttl} (rounded to {})",
        initial_ttl(ttl)
    );
    println!("✓ Linux TCP TTL: observed {ttl} (initial 64)");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_tcp_windows_distinguishable_from_linux() {
    let win_ttl = observe_ttl(Browser::Chrome147, Platform::Windows).await;
    let linux_ttl = observe_ttl(Browser::Chrome147, Platform::Linux).await;
    assert!(
        win_ttl > linux_ttl + 40,
        "Windows TTL ({win_ttl}) not sufficiently greater than Linux TTL ({linux_ttl}) — \
         TCP profile may not be applied"
    );
    println!("✓ TCP TTL distinguishes Windows ({win_ttl}) from Linux ({linux_ttl})");
}
