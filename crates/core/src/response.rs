//! HTTP response types.

use std::collections::HashMap;

/// An HTTP response with buffered body.
#[derive(Debug)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers (preserves order and multi-value headers like Set-Cookie).
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
    /// Cookies from Set-Cookie headers.
    pub cookies: HashMap<String, String>,
    /// Final URL (after redirects).
    pub url: String,
    /// URLs visited during redirect chain.
    pub redirect_chain: Vec<String>,
    /// TLS peer certificate (DER-encoded), if available.
    pub tls_peer_certificate: Option<Vec<u8>>,
}

impl Response {
    /// Response body as a UTF-8 string.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Deserialize the response body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> crate::Result<T> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Whether the status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the response was a redirect (3xx).
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    /// Get the first value for a response header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Get all values for a response header (case-insensitive).
    pub fn header_all(&self, name: &str) -> Vec<&str> {
        let lower = name.to_lowercase();
        self.headers
            .iter()
            .filter(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}
