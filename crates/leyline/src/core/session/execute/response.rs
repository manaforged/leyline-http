use std::sync::Arc;
use std::time::SystemTime;

use url::Url;

use super::super::decompress::{decompress_and_strip, drain_stream_into_vec};
use super::journey::Journey;
use crate::core::Session;
use crate::core::deadline::Deadline;
use crate::core::device::{SessionState, StateParts, unix_secs};
use crate::core::error::Result;
use crate::core::response::Response;
use crate::core::transport::{ResponseMode, TransportResponse};
use crate::util::lock;

impl Session {
    pub(super) fn store_cookies(
        &self,
        resp_headers: &[(http::HeaderName, http::HeaderValue)],
        url: &Url,
    ) {
        let set_cookies: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| *k == "set-cookie")
            .filter_map(|(_, v)| v.to_str().ok())
            .collect();
        if !set_cookies.is_empty() {
            self.inner
                .cookie_jar
                .store_response_cookies(set_cookies.as_slice(), url);
        }
    }

    #[must_use]
    pub fn state(&self) -> SessionState {
        let now = SystemTime::now();
        SessionState::from_parts(StateParts {
            tls_sessions: self.inner.connector.session_cache().export(unix_secs(now)),
            #[cfg(feature = "http3")]
            alt_svc: self.inner.pool.alt_svc_entries(),
            #[cfg(not(feature = "http3"))]
            alt_svc: Vec::new(),
            hsts: lock(&self.inner.pool.hsts).export(now),
        })
    }

    pub(crate) fn restore_state(&self, parts: &StateParts) {
        let now = SystemTime::now();
        self.inner
            .connector
            .session_cache()
            .import(&parts.tls_sessions, unix_secs(now));
        #[cfg(feature = "http3")]
        self.inner.pool.restore_alt_svc(&parts.alt_svc);
        lock(&self.inner.pool.hsts).import(&parts.hsts, now);
    }

    pub(super) fn note_hsts(
        &self,
        url: &Url,
        resp_headers: &[(http::HeaderName, http::HeaderValue)],
    ) {
        if self.inner.tls_trust.accepts_invalid_certs() {
            return;
        }
        let values: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| *k == "strict-transport-security")
            .filter_map(|(_, v)| v.to_str().ok())
            .collect();
        if !values.is_empty() {
            lock(&self.inner.pool.hsts).note(url, &values, SystemTime::now());
        }
    }

    pub(super) fn hsts_upgrade(&self, url: &Arc<Url>) -> Arc<Url> {
        lock(&self.inner.pool.hsts)
            .upgrade(url, SystemTime::now())
            .map_or_else(|| Arc::clone(url), Arc::new)
    }

    #[cfg(feature = "http3")]
    pub(super) fn note_alt_svc(
        &self,
        url: &Url,
        resp_headers: &[(http::HeaderName, http::HeaderValue)],
    ) {
        let owned: Vec<String> = resp_headers
            .iter()
            .filter(|(k, _)| *k == "alt-svc")
            .map(|(_, v)| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        let fields: Vec<&str> = owned.iter().map(String::as_str).collect();
        if fields.is_empty() {
            return;
        }
        let age = resp_headers
            .iter()
            .find(|(k, _)| *k == "age")
            .and_then(|(_, v)| v.to_str().ok())
            .and_then(|v| crate::util::delta_seconds(v.split(',').next().unwrap_or_default()))
            .map_or(std::time::Duration::ZERO, std::time::Duration::from_secs);
        if let Some(host) = url.host_str()
            && let Some(port) = url.port_or_known_default()
        {
            self.inner.pool.note_alt_svc(host, port, &fields, age);
        }
    }

    pub(super) async fn release_leg_body(
        &self,
        body: crate::core::transport::TransportBody,
        response: ResponseMode,
        deadline: &Deadline,
    ) {
        if response != ResponseMode::ErrorPrefix {
            return;
        }
        if let crate::core::transport::TransportBody::Streaming(mut bs) = body {
            bs.set_read_timeout(deadline.read());
            bs.set_body_timeout(deadline.body());
            bs.discard_prefix(
                self.inner.compression.max_error_body,
                deadline.error_body_wait(),
            )
            .await;
        }
    }

    pub(super) async fn assemble_response(
        &self,
        leg: TransportResponse,
        journey: Journey,
        audit_headers: Vec<(String, String)>,
        response: ResponseMode,
        deadline: &Deadline,
    ) -> Result<Response> {
        let TransportResponse {
            status,
            headers,
            trailers,
            body,
            final_url,
            version,
            tls,
            ..
        } = leg;
        let (body, headers) = self
            .finalize_response_body(
                body,
                headers,
                response.keeps_stream(status.as_u16()),
                deadline,
            )
            .await?;
        let audited = self.inner.audit_tls.is_some();
        let mut audit_headers = audit_headers;
        audit_headers.retain(|(name, value)| crate::core::transport::sent_on(version, name, value));
        Ok(Response {
            status,
            headers: headers.into_iter().collect(),
            body,
            url: final_url,
            redirect_chain: journey.chain,
            version,
            trailers,
            request_headers: audit_headers,
            tls,
            request_method: if audited {
                journey.method
            } else {
                String::new()
            },
            audit_tls: self.inner.audit_tls.clone(),
            audit_cache: std::sync::OnceLock::new(),
            compression: self.inner.compression,
            timing: journey.timing,
            attempts: 1,
            proxy: None,
        })
    }

    pub(super) async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: Vec<(http::HeaderName, http::HeaderValue)>,
        keep_stream: bool,
        deadline: &Deadline,
    ) -> Result<(
        crate::core::response::ResponseBody,
        Vec<(http::HeaderName, http::HeaderValue)>,
    )> {
        Ok(match resp_body_shape {
            crate::core::transport::TransportBody::Streaming(mut bs) if keep_stream => {
                bs.set_read_timeout(deadline.read());
                bs.set_body_timeout(deadline.body());
                bs.stop_on(&self.inner.shutdown);
                bs.watch_end();
                (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Streaming(mut bs) => {
                bs.set_read_timeout(deadline.read());
                bs.set_body_timeout(deadline.body());
                let buf = drain_stream_into_vec(bs, self.inner.compression.max_body_size).await?;
                let (buf, resp_headers) =
                    decompress_and_strip(buf, resp_headers, &self.inner.compression)?;
                (
                    crate::core::response::ResponseBody::Buffered(buf),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Buffered(buf) => {
                if keep_stream {
                    let mut bs =
                        crate::core::body_stream::BodyStream::from_bytes(bytes::Bytes::from(buf));
                    bs.stop_on(&self.inner.shutdown);
                    bs.watch_end();
                    (
                        crate::core::response::ResponseBody::Streaming(bs),
                        resp_headers,
                    )
                } else {
                    let (buf, resp_headers) =
                        decompress_and_strip(buf, resp_headers, &self.inner.compression)?;
                    (
                        crate::core::response::ResponseBody::Buffered(buf),
                        resp_headers,
                    )
                }
            }
        })
    }
}
