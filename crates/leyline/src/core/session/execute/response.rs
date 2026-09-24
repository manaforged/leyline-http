use url::Url;

use super::super::decompress::{decompress_and_strip, drain_stream_into_vec};
use crate::core::Session;
use crate::core::deadline::Deadline;
use crate::core::error::Result;

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
            crate::core::transport::TransportBody::Streaming(bs) => {
                let buf = deadline.read_body(drain_stream_into_vec(bs)).await?;
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
