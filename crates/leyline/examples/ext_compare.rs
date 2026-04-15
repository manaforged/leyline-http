//! Print the observed TLS extension list for a Chrome-profiled request.

use leyline::Session;

#[tokio::main]
async fn main() {
    let session = Session::chrome().expect("failed to build session");
    let resp = session
        .navigate("https://tls.peet.ws/api/all")
        .await
        .expect("request failed");
    let v: serde_json::Value = resp.json().expect("invalid json");

    println!(
        "Extensions we send ({}):",
        v["tls"]["extensions"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0)
    );
    for ext in v["tls"]["extensions"].as_array().unwrap() {
        let name = ext["name"].as_str().unwrap_or("?");
        println!("  {name}");
    }

    println!();
    println!("Chrome 147 should send (16):");
    println!("  GREASE");
    println!("  server_name (0)");
    println!("  extended_master_secret (23)");
    println!("  extensionRenegotiationInfo (65281)");
    println!("  supported_groups (10)");
    println!("  ec_point_formats (11)");
    println!("  session_ticket (35)");
    println!("  application_layer_protocol_negotiation (16)");
    println!("  status_request (5)");
    println!("  signature_algorithms (13)");
    println!("  signed_certificate_timestamp (18)");
    println!("  key_share (51)");
    println!("  psk_key_exchange_modes (45)");
    println!("  supported_versions (43)");
    println!("  compress_certificate (27)");
    println!("  encrypted_client_hello (GREASE ECH)");
    println!("  GREASE");
}
