use std::sync::Arc;
use std::time::SystemTime;

use url::Url;

use super::super::decompress::{buffer_and_strip, decompress_and_strip};
use super::super::helpers::header_values;
use super::journey::Journey;
use crate::core::BodyRules;
use crate::core::Session;
use crate::core::deadline::Deadline;
use crate::core::error::Result;
use crate::core::response::Response;
use crate::core::saved::{ConnectionState, StateParts};
use crate::core::transport::{ResponseMode, TransportResponse};
use crate::util::lock;
use crate::util::since_epoch;

impl Session {
    pub(super) fn store_cookies(&self, resp_headers: &http::HeaderMap, url: &Url) {
        let set_cookies: Vec<&str> = header_values(resp_headers, http::header::SET_COOKIE)
            .filter_map(|v| v.to_str().ok())
            .collect();
        if !set_cookies.is_empty() {
            self.inner
                .cookie_jar
                .store_response_cookies(set_cookies.as_slice(), url);
        }
    }

    bench_pub! {
    #[must_use]
    fn connection_state(&self) -> ConnectionState {
        let now = SystemTime::now();
        ConnectionState::from_parts(StateParts {
            tls_sessions: self.inner.connector.session_cache().export(since_epoch(now).as_secs()),
            #[cfg(feature = "http3")]
            alt_svc: self.inner.pool.alt_svc_entries(),
            #[cfg(not(feature = "http3"))]
            alt_svc: Vec::new(),
            hsts: lock(&self.inner.pool.hsts).export(now),
        })
    }
    }

    bench_pub! {
    fn restore(&self, state: &ConnectionState) {
        self.restore_state(&state.parts());
    }
    }

    pub(crate) fn restore_state(&self, parts: &StateParts) {
        let now = SystemTime::now();
        self.inner
            .connector
            .session_cache()
            .import(&parts.tls_sessions, since_epoch(now).as_secs());
        #[cfg(feature = "http3")]
        self.inner.pool.restore_alt_svc(&parts.alt_svc);
        lock(&self.inner.pool.hsts).import(&parts.hsts, now);
    }

    pub(super) fn note_hsts(&self, url: &Url, resp_headers: &http::HeaderMap) {
        if self.inner.tls_trust.accepts_invalid_certs() {
            return;
        }
        let values: Vec<&str> =
            header_values(resp_headers, http::header::STRICT_TRANSPORT_SECURITY)
                .filter_map(|v| v.to_str().ok())
                .collect();
        if !values.is_empty() {
            lock(&self.inner.pool.hsts).note(url, &values, SystemTime::now());
        }
    }

    pub(in crate::core::session) fn hsts_upgrade(&self, url: &Arc<Url>) -> Arc<Url> {
        if !crate::core::hsts::HstsStore::applies(url) {
            return Arc::clone(url);
        }
        lock(&self.inner.pool.hsts)
            .upgrade(url, SystemTime::now())
            .map_or_else(|| Arc::clone(url), Arc::new)
    }

    #[cfg(feature = "http3")]
    pub(super) fn note_alt_svc(&self, url: &Url, resp_headers: &http::HeaderMap) {
        let owned: Vec<String> = header_values(resp_headers, http::header::ALT_SVC)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        let fields: Vec<&str> = owned.iter().map(String::as_str).collect();
        if fields.is_empty() {
            return;
        }
        let age = header_values(resp_headers, http::header::AGE)
            .next()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| crate::util::delta_seconds(v.split(',').next().unwrap_or_default()))
            .map_or(std::time::Duration::ZERO, std::time::Duration::from_secs);
        if let Some(host) = url.host_str()
            && let Some(port) = url.port_or_known_default()
        {
            self.inner.pool.note_alt_svc(host, port, &fields, age);
        }
    }

    pub(super) async fn assemble_response(
        &self,
        leg: TransportResponse,
        journey: Journey,
        audit_headers: Vec<(String, String)>,
        response: ResponseMode,
        rules: BodyRules,
        deadline: &Deadline,
    ) -> Result<Response> {
        let TransportResponse {
            status,
            headers,
            trailers,
            body,
            version,
            tls,
            ..
        } = leg;
        let (body, headers) = self
            .finalize_response_body(
                body,
                headers,
                response.keeps_stream(status.as_u16()),
                &rules,
                deadline,
            )
            .await?;
        let audited = self.inner.audit_tls.is_some();
        let mut audit_headers = audit_headers;
        audit_headers.retain(|(name, value)| crate::core::transport::sent_on(version, name, value));
        let mut response = Response::from_parts(crate::core::response::Parts {
            status,
            headers,
            body,
            url: journey.url,
            redirect_chain: journey.chain,
            version,
            trailers,
            request_headers: audit_headers,
            tls,
            request_method: if audited {
                journey.method.to_string()
            } else {
                String::new()
            },
            audit_tls: self.inner.audit_tls.clone(),
            audit_cache: std::sync::OnceLock::new(),
            compression: rules,
            timing: journey.timing,
            attempts: 1,
            proxy: None,
            block: None,
        });
        response.parts.block = self.inner.block_rules.check(&response);
        Ok(response)
    }

    pub(super) async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: http::HeaderMap,
        keep_stream: bool,
        rules: &BodyRules,
        deadline: &Deadline,
    ) -> Result<(crate::core::response::ResponseBody, http::HeaderMap)> {
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
            crate::core::transport::TransportBody::Streaming(bs) => {
                let mut bs = *bs;
                bs.set_read_timeout(deadline.read());
                bs.set_body_timeout(deadline.body());
                let (buf, resp_headers) = buffer_and_strip(bs, resp_headers, rules).await?;
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
                        crate::core::response::ResponseBody::Streaming(Box::new(bs)),
                        resp_headers,
                    )
                } else if !resp_headers.contains_key(http::header::CONTENT_ENCODING) {
                    (
                        crate::core::response::ResponseBody::Buffered(buf),
                        resp_headers,
                    )
                } else {
                    let (buf, resp_headers) = decompress_and_strip(buf, resp_headers, rules)?;
                    (
                        crate::core::response::ResponseBody::Buffered(buf),
                        resp_headers,
                    )
                }
            }
        })
    }
}
