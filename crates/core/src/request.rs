//! Fluent request builder.

use std::collections::HashMap;

use leyline_profile::Preset;

use crate::response::Response;
use crate::session::Session;
use crate::Result;

/// Fluent builder for constructing and sending HTTP requests.
pub struct RequestBuilder<'a> {
    session: &'a Session,
    method: String,
    url: String,
    preset: Option<Preset>,
    body: Option<Vec<u8>>,
    headers: HashMap<String, String>,
}

impl<'a> RequestBuilder<'a> {
    pub(crate) fn new(session: &'a Session, method: &str, url: &str) -> Self {
        Self {
            session,
            method: method.to_string(),
            url: url.to_string(),
            preset: None,
            body: None,
            headers: HashMap::new(),
        }
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

    /// Set the request body as JSON.
    pub fn json(mut self, value: &impl serde::Serialize) -> Result<Self> {
        let bytes = serde_json::to_vec(value)?;
        self.headers
            .insert("content-type".to_string(), "application/json".to_string());
        self.body = Some(bytes);
        Ok(self)
    }

    /// Set a request header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    /// Send the request and return a buffered response.
    pub async fn send(self) -> Result<Response> {
        self.session
            .execute(
                &self.method,
                &self.url,
                self.preset,
                self.body,
                if self.headers.is_empty() {
                    None
                } else {
                    Some(self.headers)
                },
            )
            .await
    }
}
