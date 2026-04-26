//! TLS fingerprint diagnostic — compare leyline Chrome 147 against tls.peet.ws.
#![allow(missing_docs)]
use leyline::profile::Browser;
use leyline::Session;

#[tokio::main]
async fn main() {
    for browser in [Browser::Chrome147, Browser::Chrome146, Browser::Chrome145] {
        eprintln!("\n=== {browser:?} ===");
        let session = Session::builder().browser(browser).build().expect("build");
        let resp = session
            .get("https://tls.browserleaks.com/json")
            .send()
            .await
            .expect("fetch");
        let body = resp.text();
        eprintln!("  RAW: {body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("parse json");
        // Trim to the fingerprint-relevant fields so we can eyeball the diff.
        let ja3 = json.pointer("/ja3").and_then(|v| v.as_str()).unwrap_or("?");
        let ja3_hash = json
            .pointer("/ja3_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let ja4 = json.pointer("/ja4").and_then(|v| v.as_str()).unwrap_or("?");
        let peetprint = json
            .pointer("/peetprint")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let peetprint_hash = json
            .pointer("/peetprint_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let h2_fp = json
            .pointer("/akamai_fingerprint")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let h2_hash = json
            .pointer("/akamai_fingerprint_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let ua = json
            .pointer("/user_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        eprintln!("  user_agent:  {ua}");
        eprintln!("  ja3:         {ja3}");
        eprintln!("  ja3_hash:    {ja3_hash}");
        eprintln!("  ja4:         {ja4}");
        eprintln!("  peetprint:   {peetprint}");
        eprintln!("  peet_hash:   {peetprint_hash}");
        eprintln!("  h2_akamai:   {h2_fp}");
        eprintln!("  h2_hash:     {h2_hash}");
    }
}
