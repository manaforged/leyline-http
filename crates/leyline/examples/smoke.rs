use leyline::Session;

#[tokio::main]
async fn main() {
    let session = Session::chrome().expect("failed to build session");

    match session.navigate("https://tls.peet.ws/api/all").await {
        Ok(resp) => {
            let v: serde_json::Value = resp.json().expect("invalid json");
            let ja4 = v["tls"]["ja4"].as_str().unwrap_or("?");
            let h2 = v["http2"]["akamai_fingerprint"].as_str().unwrap_or("?");
            let ext_count = v["tls"]["extensions"].as_array().map(|a| a.len()).unwrap_or(0);
            let scheme = v["http2"]["sent_frames"][2]["headers"]
                .as_array()
                .and_then(|h| h.iter().find(|s| s.as_str().unwrap_or("").starts_with(":scheme")))
                .and_then(|s| s.as_str())
                .unwrap_or("?");

            println!("JA4:  {ja4}");
            println!("H2:   {h2}");
            println!("Ext:  {ext_count}");
            println!("Scheme: {scheme}");

            // Expected values
            println!();
            println!("Expected JA4:  t13d1516h2_8daaf6152771_d8a2da3f94cd");
            println!("Expected H2:   1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
