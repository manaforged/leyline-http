//! Transport layer — connects TLS/plain TCP and sends requests.
//!
//! HTTPS defaults to our own leyline-h2 implementation with connection pooling.
//! Plain `http://` and explicit H1 policy go through the HTTP/1.1
//! keep-alive pool in [`crate::pool`].

use std::sync::Arc;

use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::pool::{H1Body, H1PooledError, H1ResponseBody, H1Target, Pool};
use crate::tls::ConnectorVariant;
use crate::util::{base64_encode, percent_decode};
use bytes::Bytes;
use futures_util::StreamExt;

use crate::core::body::Body;
use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Result};
use crate::core::response::HttpVersion;

const MAX_H1_BODY_BYTES: usize = 100 * 1024 * 1024;

/// Body shape returned by a transport. Either fully buffered, or a
/// receiver the caller drains via `BodyStream`.
pub(crate) enum TransportBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
}

/// Response returned by a transport.
pub(crate) struct TransportResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: TransportBody,
    pub(crate) final_url: String,
    pub(crate) version: HttpVersion,
    pub(crate) tls_alpn: Option<String>,
    pub(crate) peer_cert_der: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
}

/// Send an HTTP request with browser-compatible defaults.
#[tracing::instrument(
    name = "transport.auto",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
        proxied = proxy.is_some(),
    )
)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_request_auto(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<TransportResponse> {
    if url.scheme() == "http" {
        return send_request_h1(
            pool,
            connector,
            method,
            url,
            headers,
            body,
            proxy,
            stream_response,
        )
        .await;
    }

    // To support the H1 fallback on ALPN mismatch we need to retain the
    // body. Streaming bodies are one-shot, so eagerly materialise them.
    // Callers who want hard streaming over H2 should pin `.http2()`.
    let (h2_body, fallback_buf): (Body, Option<Bytes>) = if body.is_stream() {
        let buf = materialise_stream_body(body).await?;
        (Body::from(buf.clone()), Some(buf))
    } else {
        match body {
            Body::Empty => (Body::Empty, None),
            Body::Bytes(b) => (Body::Bytes(b.clone()), Some(b)),
            Body::Stream { .. } => unreachable!("stream branch handled above"),
        }
    };

    match send_request_h2(
        pool,
        connector,
        h2_config,
        method,
        url,
        headers.clone(),
        h2_body,
        proxy,
        stream_response,
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(e) if is_h2_alpn_mismatch(&e) => {
            tracing::debug!(error = %e, "H2 ALPN mismatch, falling back to HTTP/1.1");
            let fallback_body = match fallback_buf {
                Some(buf) => Body::from(buf),
                None => Body::Empty,
            };
            send_request_h1(
                pool,
                connector,
                method,
                url,
                headers,
                fallback_body,
                proxy,
                stream_response,
            )
            .await
        }
        Err(e) => Err(e),
    }
}

/// Drain a streaming body into a single `Bytes` buffer. Used when the
/// transport can't accept streams (H3) or needs to retain the body for
/// a fallback retry.
async fn materialise_stream_body(body: Body) -> Result<Bytes> {
    match body {
        Body::Empty => Ok(Bytes::new()),
        Body::Bytes(b) => Ok(b),
        Body::Stream { mut stream, .. } => {
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(Error::Io)?;
                buf.extend_from_slice(&chunk);
                if buf.len() > MAX_H1_BODY_BYTES {
                    return Err(Error::Body(format!(
                        "streaming request body exceeded {MAX_H1_BODY_BYTES} bytes"
                    )));
                }
            }
            Ok(Bytes::from(buf))
        }
    }
}

/// Send an HTTP request, reusing pooled H2 connections when available.
#[tracing::instrument(
    name = "transport.h2",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_request_h2(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    stream_response: bool,
) -> Result<TransportResponse> {
    if url.scheme() != "https" {
        return Err(Error::Config("HTTP/2 requires an https:// URL".into()));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    // Build pseudo-headers.
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let pseudo = PseudoHeaders {
        method: method.to_string(),
        scheme: url.scheme().to_string(),
        authority: {
            let is_default_port =
                (url.scheme() == "https" && port == 443) || (url.scheme() == "http" && port == 80);
            if is_default_port {
                host.to_string()
            } else {
                format!("{host}:{port}")
            }
        },
        path: format!("{path}{query}"),
        protocol: None,
    };

    // Translate Body → h2 request body representation.
    let h2_req_body = body_to_h2_request(body);

    // Send via pool (reuses connection or creates new one).
    let (resp, tls) = crate::pool::send_request(
        pool,
        connector,
        h2_config,
        pseudo,
        headers,
        h2_req_body,
        proxy,
        stream_response,
    )
    .await?;

    let transport_body = match resp.body {
        crate::h2::client::ResponseBody::Buffered(b) => TransportBody::Buffered(b),
        crate::h2::client::ResponseBody::Streaming(rx) => {
            TransportBody::Streaming(BodyStream::new(rx))
        }
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: transport_body,
        final_url: url.to_string(),
        version: HttpVersion::Http2,
        tls_alpn: Some("h2".to_string()),
        peer_cert_der: tls.peer_cert_der,
        tls_version: tls.version,
        tls_cipher: tls.cipher,
    })
}

/// Translate a [`Body`] into the h2-crate request body shape.
fn body_to_h2_request(body: Body) -> crate::h2::client::RequestBody {
    match body {
        Body::Empty => crate::h2::client::RequestBody::None,
        Body::Bytes(b) => crate::h2::client::RequestBody::Buffered(b),
        Body::Stream {
            stream,
            length_hint,
        } => crate::h2::client::RequestBody::Streaming {
            stream,
            length_hint,
        },
    }
}

/// Translate a [`Body`] into the H1 pool's request body shape.
fn body_to_h1(body: Body) -> H1Body {
    match body {
        Body::Empty => H1Body::Empty,
        Body::Bytes(b) => H1Body::Buffered(b),
        Body::Stream {
            stream,
            length_hint: Some(length),
        } => H1Body::FixedStream { stream, length },
        Body::Stream {
            stream,
            length_hint: None,
        } => H1Body::ChunkedStream { stream },
    }
}

/// Send an HTTP/1.1 request through the HTTP/1.1 keep-alive pool.
///
/// HTTPS uses `connector.connect_h1` (BoringSSL with `http/1.1` ALPN);
/// plaintext HTTP uses a bare `TcpStream`. Connections are reused for
/// subsequent requests to the same `(host, port, proxy)` destination
/// when the response allows keep-alive.
#[tracing::instrument(
    name = "transport.h1",
    level = "debug",
    skip_all,
    fields(
        http.method = method,
        http.scheme = url.scheme(),
        http.host = url.host_str().unwrap_or(""),
    )
)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_request_h1(
    pool: &Arc<Pool>,
    connector: &ConnectorVariant,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    proxy: Option<&str>,
    // H1 response bodies are buffered today; `.stream()` callers still get a
    // working `Response::into_stream()` (the buffer is handed back as one
    // chunk). Memory-incremental H1 streaming (chunk-by-chunk pump off a
    // dedicated, non-pooled connection) is a scoped follow-up — see the
    // generalist-gap plan. The flag is accepted here so the signature is
    // uniform with the H2 path.
    _stream_response: bool,
) -> Result<TransportResponse> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::Config(format!("no default port for scheme {}", url.scheme())))?;
    let scheme = url.scheme();

    // Plaintext HTTP via an http proxy uses absolute-form request
    // targets (`GET http://host/path HTTP/1.1`) and may carry
    // `Proxy-Authorization`. HTTPS through a proxy is already handled
    // inside `connector.connect_h1` via the CONNECT tunnel.
    let (target, headers) = match (scheme, proxy) {
        ("http", Some(proxy_url)) => {
            let parsed = url::Url::parse(proxy_url)
                .map_err(|e| Error::Config(format!("invalid proxy URL: {e}")))?;
            if parsed.scheme() != "http" {
                return Err(Error::Config(
                    "plaintext HTTP currently supports http:// proxies only".into(),
                ));
            }
            let mut headers = headers;
            if let Some(password) = parsed.password() {
                let credentials = base64_encode(&format!(
                    "{}:{}",
                    percent_decode(parsed.username()),
                    percent_decode(password)
                ));
                headers.push(("Proxy-Authorization".into(), format!("Basic {credentials}")));
            }
            (H1Target::AbsoluteForm, headers)
        }
        _ => (H1Target::OriginForm, headers),
    };

    let h1_body = body_to_h1(body);

    let resp = crate::pool::send_request_h1_pooled(
        pool, connector, scheme, host, port, method, url, headers, h1_body, proxy, target,
    )
    .await
    .map_err(h1_error_to_core)?;

    let H1ResponseBody::Buffered(body_buf) = resp.body;

    let (tls_alpn, peer_cert_der, tls_version, tls_cipher) = match resp.tls {
        Some(info) => (
            // The H1 path always negotiates http/1.1 when TLS is
            // involved; surface it for audit. For plaintext HTTP
            // everything below is `None`.
            Some("http/1.1".to_string()),
            info.peer_cert_der,
            info.version,
            info.cipher,
        ),
        None => (None, None, None, None),
    };

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: TransportBody::Buffered(body_buf),
        final_url: url.to_string(),
        version: HttpVersion::Http1_1,
        tls_alpn,
        peer_cert_der,
        tls_version,
        tls_cipher,
    })
}

fn h1_error_to_core(e: H1PooledError) -> Error {
    match e {
        H1PooledError::Config(m) => Error::Config(m),
        H1PooledError::Tls(m) => Error::Tls(crate::tls::TlsError::SslConnect(m)),
        H1PooledError::Io(io) => Error::Io(io),
        H1PooledError::Http(m) => Error::Http(m),
        // Transport-level EOF mid-exchange → typed Io(UnexpectedEof), so the
        // retry engine treats it as a connection error via the typed `Io` arm
        // (no substring sniffing of an attacker-influenceable message).
        H1PooledError::ConnectionClosed(ctx) => Error::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!("connection closed {ctx}"),
        )),
    }
}

/// Send an HTTP/3 request over QUIC.
#[cfg(feature = "http3")]
#[tracing::instrument(
    name = "transport.h3",
    level = "debug",
    skip_all,
    fields(http.method = method, http.host = url.host_str().unwrap_or(""))
)]
pub(crate) async fn send_request_h3(
    h3_config: &crate::quic::H3Config,
    profile: &crate::profile::BrowserProfile,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Body,
    stream_response: bool,
) -> Result<TransportResponse> {
    // H3 incremental streaming (quiche-level pump/pull) is deferred. A
    // streaming REQUEST body still can't be sent as a stream, so reject that
    // explicitly. A `.stream()` RESPONSE request, however, degrades
    // gracefully: we buffer the body and hand it back through the same
    // `into_stream()` surface (as a single chunk) rather than hard-erroring,
    // so `.stream()` behaves consistently across H1/H2/H3 for callers.
    if body.is_stream() {
        return Err(Error::Config(
            "HTTP/3 streaming request bodies are not yet implemented; use .http2() or buffer \
             the body before sending"
                .into(),
        ));
    }
    if stream_response {
        tracing::debug!(
            target: "leyline::h3",
            "response streaming over HTTP/3 is not yet incremental; buffering the body \
             (it is still available via Response::into_stream())"
        );
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let path = url.path();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let full_path = format!("{path}{query}");

    let body_bytes = match body {
        Body::Empty => None,
        Body::Bytes(b) => Some(b),
        Body::Stream { .. } => unreachable!("rejected above"),
    };

    let resp = crate::quic::H3Connection::request(
        h3_config, profile, method, host, port, &full_path, headers, body_bytes,
    )
    .await
    .map_err(Error::Http3)?;

    Ok(TransportResponse {
        status: resp.status,
        headers: resp.headers,
        body: TransportBody::Buffered(resp.body),
        final_url: url.to_string(),
        version: HttpVersion::Http3,
        tls_alpn: Some("h3".to_string()),
        // H3/QUIC peer-cert extraction and TLS details are a separate
        // piece of work — the quiche path doesn't currently expose the
        // handshake result through the connection handle.
        peer_cert_der: None,
        tls_version: None,
        tls_cipher: None,
    })
}

/// True when the H2 attempt failed because the server declined the `h2` ALPN
/// (e.g. some CDN/WAF edges serve a cookieless interstitial over HTTP/1.1,
/// replying with no ALPN). Real Chrome falls back to HTTP/1.1 in this case, so
/// the transport retries on h1 instead of erroring out.
///
/// The pool path surfaces this as `Error::AlpnMismatch` (see
/// `pool::open_fresh_h2`); the transport keys the HTTP/1.1 fallback off that
/// typed variant so an unrelated H2 or HTTP error never trips it.
fn is_h2_alpn_mismatch(err: &Error) -> bool {
    matches!(err, Error::AlpnMismatch { .. })
}

#[cfg(test)]
mod alpn_fallback_tests {
    use super::*;

    #[test]
    fn alpn_mismatch_detected() {
        // The pool surfaces the ALPN decline as the typed Error::AlpnMismatch,
        // which must trip the HTTP/1.1 fallback regardless of which protocol
        // (or none) the peer negotiated. Without it, proxyless navigations to
        // hosts that decline h2 ALPN on a cookieless interstitial hard-error
        // instead of falling back.
        assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
            negotiated: "none".into()
        }));
        assert!(is_h2_alpn_mismatch(&Error::AlpnMismatch {
            negotiated: "http/1.1".into()
        }));
        // Unrelated errors must NOT trigger a fallback — neither a generic
        // HTTP error nor a non-ALPN HTTP/2 transport error.
        assert!(!is_h2_alpn_mismatch(&Error::Http("404 not found".into())));
        assert!(!is_h2_alpn_mismatch(&Error::Http2(
            crate::h2::H2Error::Stream {
                stream_id: 1,
                code: crate::h2::error::ErrorCode::RefusedStream,
            }
        )));
    }

    #[test]
    fn h1_connection_closed_is_typed_retryable_but_framing_is_not() {
        use crate::pool::H1PooledError;
        // Mid-exchange EOF → typed Io(UnexpectedEof), classified as a
        // connection-closed (so the retry engine's typed Io arm retries it).
        let closed = h1_error_to_core(H1PooledError::ConnectionClosed("before headers".into()));
        assert!(
            matches!(&closed, Error::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof),
            "ConnectionClosed must map to Io(UnexpectedEof), got {closed:?}"
        );
        assert!(closed.is_connection_closed());
        // A framing error whose message interpolates attacker-controlled header
        // bytes containing "connection" must NOT be classified as
        // connection-closed — the old substring match would have retried it
        // (a request-smuggling hazard); the typed path does not.
        let framing = h1_error_to_core(H1PooledError::Http(
            "invalid Transfer-Encoding: connection-close".into(),
        ));
        assert!(matches!(framing, Error::Http(_)));
        assert!(!framing.is_connection_closed());
    }
}
