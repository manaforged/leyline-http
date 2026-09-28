use super::{TransportBody, TransportResponse, adopt, status};
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::response::{HttpVersion, ResponseTiming};
use crate::h2::client::{H2ResponseEx, ResponseBody};
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::TlsInfo;
use crate::util::request_target;

const STANDARD_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "PATCH", "TRACE", "CONNECT",
];

fn method_header(method: &str) -> HeaderStr {
    STANDARD_METHODS
        .iter()
        .copied()
        .find(|known| *known == method)
        .map_or_else(|| HeaderStr::from(method), HeaderStr::from_static)
}

fn authority_header(host: &str, port: u16) -> HeaderStr {
    if port == 443 {
        HeaderStr::from(host)
    } else {
        HeaderStr::from(format!("{host}:{port}"))
    }
}

pub(super) fn request_pseudo(method: &str, url: &url::Url) -> Result<PseudoHeaders> {
    if url.scheme() != "https" {
        return Err(Error::new(Kind::Config).with_message("HTTP/2 requires an https:// URL"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::new(Kind::Config).with_message("no host in URL"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    Ok(PseudoHeaders {
        method: method_header(method),
        scheme: HeaderStr::from_static("https"),
        authority: authority_header(host, port),
        path: HeaderStr::from(request_target(url)),
        protocol: None,
    })
}

pub(super) fn transport_response(
    resp: H2ResponseEx,
    tls: TlsInfo,
    timing: ResponseTiming,
    url: &url::Url,
) -> Result<TransportResponse> {
    let body = match resp.body {
        ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        ResponseBody::Streaming(rx) => TransportBody::Streaming(BodyStream::new(rx)),
    };
    Ok(TransportResponse {
        status: status(resp.status)?,
        headers: adopt(resp.headers),
        trailers: adopt(resp.trailers.unwrap_or_default()),
        body,
        final_url: url.clone(),
        version: HttpVersion::Http2,
        tls: Some(tls),
        timing,
    })
}
