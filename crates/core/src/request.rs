//! Fluent request builder.

use std::time::Duration;

use leyline_profile::Preset;

use crate::error::Error;
use crate::headers::HeaderList;
use crate::response::Response;
use crate::session::Session;
use crate::Result;

/// Fluent builder for constructing and sending HTTP requests.
///
/// ```rust,ignore
/// let resp = session.post("https://api.example.com/items")
///     .preset(Preset::Xhr)
///     .json(&payload)
///     .bearer_auth("token123")
///     .header("x-request-id", "abc")
///     .send()
///     .await?;
/// ```
pub struct RequestBuilder<'a> {
    session: &'a Session,
    method: String,
    url: String,
    preset: Option<Preset>,
    body: Option<Vec<u8>>,
    headers: HeaderList,
    query_params: Vec<(String, String)>,
    timeout: Option<Duration>,
    builder_error: Option<Error>,
}

impl<'a> RequestBuilder<'a> {
    pub(crate) fn new(session: &'a Session, method: &str, url: &str) -> Self {
        Self {
            session,
            method: method.to_string(),
            url: url.to_string(),
            preset: None,
            body: None,
            headers: HeaderList::new(),
            query_params: Vec::new(),
            timeout: None,
            builder_error: None,
        }
    }

    /// Override the session's default timeout for this one request.
    ///
    /// ```rust,ignore
    /// session.post(url)
    ///     .json(&payload)
    ///     .timeout(Duration::from_secs(5))
    ///     .send()
    ///     .await?;
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set a request preset (Navigate, Script, Xhr, Form, CrossOrigin, SameSite).
    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Set the request body as raw bytes.
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set the request body as JSON. Sets `content-type: application/json`.
    ///
    /// Serialization errors are deferred: the error is stashed and returned
    /// by [`send`](Self::send), so chaining stays panic-free.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.headers.set("content-type", "application/json");
                self.body = Some(bytes);
            }
            Err(e) => {
                self.builder_error = Some(Error::Json(e));
            }
        }
        self
    }

    /// Set the request body as URL-encoded form data. Sets content-type automatically.
    ///
    /// ```rust,ignore
    /// session.post(url).form(&[("user", "alice"), ("pass", "secret")]).send().await?;
    /// ```
    pub fn form(mut self, params: &[(&str, &str)]) -> Self {
        let encoded = url_encode_pairs(params);
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Some(encoded.into_bytes());
        self
    }

    /// Set the request body as a pre-encoded form string. Sets content-type automatically.
    pub fn form_str(mut self, encoded: &str) -> Self {
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Some(encoded.as_bytes().to_vec());
        self
    }

    /// Add URL query parameters. Can be called multiple times.
    ///
    /// ```rust,ignore
    /// session.get(url)
    ///     .query(&[("page", "2"), ("sort", "price")])
    ///     .send().await?;
    /// ```
    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        for &(k, v) in params {
            self.query_params.push((k.to_string(), v.to_string()));
        }
        self
    }

    /// Set a request header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Append a request header without replacing existing values with the same name.
    pub fn append_header(mut self, name: &str, value: &str) -> Self {
        self.headers.append(name, value);
        self
    }

    /// Set multiple headers at once.
    pub fn headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.set(k, v);
        }
        self
    }

    /// Append multiple headers, preserving duplicate names and order.
    pub fn append_headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.append(k, v);
        }
        self
    }

    /// Set a Bearer token for the Authorization header.
    ///
    /// ```rust,ignore
    /// session.get(url).bearer_auth("my-jwt-token").send().await?;
    /// ```
    pub fn bearer_auth(mut self, token: &str) -> Self {
        self.headers.set("authorization", format!("Bearer {token}"));
        self
    }

    /// Set Basic auth for the Authorization header.
    ///
    /// ```rust,ignore
    /// session.get(url).basic_auth("user", "pass").send().await?;
    /// ```
    pub fn basic_auth(mut self, username: &str, password: &str) -> Self {
        let encoded = base64_encode(&format!("{username}:{password}"));
        self.headers
            .set("authorization", format!("Basic {encoded}"));
        self
    }

    /// Send the request and return a buffered response.
    pub async fn send(mut self) -> Result<Response> {
        if let Some(err) = self.builder_error {
            return Err(err);
        }

        // Append query params to URL.
        if !self.query_params.is_empty() {
            let mut url = url::Url::parse(&self.url)?;
            {
                let mut pairs = url.query_pairs_mut();
                for (k, v) in &self.query_params {
                    pairs.append_pair(k, v);
                }
            }
            self.url = url.to_string();
        }

        let headers = if self.headers.is_empty() {
            None
        } else {
            Some(self.headers)
        };

        self.session
            .execute_with_timeout(
                &self.method,
                &self.url,
                self.preset,
                self.body,
                headers,
                self.timeout,
            )
            .await
    }
}

/// URL-encode key-value pairs.
pub(crate) fn url_encode_pairs(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode a string for URL form data.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(HEX_UPPER[(b >> 4) as usize] as char);
                out.push(HEX_UPPER[(b & 0xF) as usize] as char);
            }
        }
    }
    out
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Simple base64 encoding for auth headers.
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(n >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(n >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_simple() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(url_encode("safe-string_v2.0"), "safe-string_v2.0");
    }

    #[test]
    fn url_encode_pairs_works() {
        let pairs = url_encode_pairs(&[("user", "alice"), ("pass", "s3cr3t!")]);
        assert_eq!(pairs, "user=alice&pass=s3cr3t%21");
    }

    #[test]
    fn base64_encode_works() {
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
    }
}
