use url::Url;

use super::super::decompress::{decompress_and_strip, drain_stream_into_vec};
use super::journey::Journey;
use crate::core::Session;
use crate::core::deadline::Deadline;
use crate::core::error::Result;
use crate::core::response::Response;
use crate::core::transport::TransportResponse;

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

    pub(super) async fn assemble_response(
        &self,
        leg: TransportResponse,
        journey: Journey,
        audit_headers: Vec<(String, String)>,
        stream_response: bool,
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
            .finalize_response_body(body, headers, stream_response, deadline)
            .await?;
        let audited = self.inner.audit_tls.is_some();
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
        })
    }

    pub(super) async fn finalize_response_body(
        &self,
        resp_body_shape: crate::core::transport::TransportBody,
        resp_headers: Vec<(http::HeaderName, http::HeaderValue)>,
        stream_response: bool,
        deadline: &Deadline,
    ) -> Result<(
        crate::core::response::ResponseBody,
        Vec<(http::HeaderName, http::HeaderValue)>,
    )> {
        Ok(match resp_body_shape {
            crate::core::transport::TransportBody::Streaming(mut bs) if stream_response => {
                bs.set_read_timeout(deadline.read());
                (
                    crate::core::response::ResponseBody::Streaming(bs),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Streaming(mut bs) => {
                bs.set_read_timeout(deadline.read());
                let buf = drain_stream_into_vec(bs, self.inner.compression.max_body_size).await?;
                let (buf, resp_headers) =
                    decompress_and_strip(buf, resp_headers, &self.inner.compression)?;
                (
                    crate::core::response::ResponseBody::Buffered(buf),
                    resp_headers,
                )
            }
            crate::core::transport::TransportBody::Buffered(buf) => {
                if stream_response {
                    (
                        crate::core::response::ResponseBody::Streaming(
                            crate::core::body_stream::BodyStream::from_bytes(bytes::Bytes::from(
                                buf,
                            )),
                        ),
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
