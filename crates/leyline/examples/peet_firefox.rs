//! Live TLS/H2 fingerprint probe — does leyline actually EMIT a real Firefox
//! fingerprint (vs. the toml's static claim)? Probes Firefox152/151/150 against
//! tls.peet.ws and prints the
//! JA4/H2 next to the real-browser values captured on the Windows box.
#![allow(missing_docs)]
use leyline::Session;
use leyline::profile::Browser;

#[tokio::main]
async fn main() {
    for browser in [
        Browser::Firefox152,
        Browser::Firefox151,
        Browser::Firefox150,
    ] {
        eprintln!("\n=== {browser:?} ===");
        let session = match Session::builder().browser(browser).build() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  BUILD ERR: {e}");
                continue;
            }
        };
        let resp = match session.get("https://tls.peet.ws/api/all").send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  FETCH ERR: {e}");
                continue;
            }
        };
        let body = resp.text();
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("  JSON ERR: {e}; raw={}", &body[..body.len().min(300)]);
                continue;
            }
        };
        let p = |ptr: &str| -> String {
            json.pointer(ptr)
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string()
        };
        eprintln!("  user_agent:       {}", p("/user_agent"));
        eprintln!("  ja4:              {}", p("/tls/ja4"));
        eprintln!("  ja4_r:            {}", p("/tls/ja4_r"));
        eprintln!("  peetprint_hash:   {}", p("/tls/peetprint_hash"));
        eprintln!("  akamai_h2:        {}", p("/http2/akamai_fingerprint"));
        eprintln!(
            "  akamai_h2_hash:   {}",
            p("/http2/akamai_fingerprint_hash")
        );
    }
    eprintln!(
        "\n--- real FF151/FF152 ja4 = t13d1617h2_86a278354501_3cbfd9057e0d ---"
    );
    eprintln!("--- real FF150 ja4       = t13d1717h2_5b57614c22b0_3cbfd9057e0d ---");
    eprintln!("--- real FF akamai_h2    = 1:65536;2:0;4:131072;5:16384|12517377|0|m,p,a,s ---");
}
