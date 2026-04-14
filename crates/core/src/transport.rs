//! Transport layer — connects TLS, sets up H2, sends requests.

use std::collections::HashMap;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper2::body::Incoming;
use hyper2::client::conn::http2;

use leyline_h2::H2Config;
use leyline_profile::BrowserProfile;
use leyline_tls::{FingerprintConnector, TlsStream};

use crate::error::{Error, Result};
use crate::response::Response;

/// Hyper2 executor backed by tokio.
#[derive(Clone)]
struct TokioExecutor;

impl<F> hyper2::rt::Executor<F> for TokioExecutor
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, fut: F) {
        tokio::spawn(fut);
    }
}

/// Adapter to make tokio_boring2::SslStream work with hyper2.
struct TokioIo<T>(T);

impl<T: tokio::io::AsyncRead + Unpin> hyper2::rt::Read for TokioIo<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        mut buf: hyper2::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let n = unsafe {
            let dst = buf.as_mut();
            let slice = &mut *(dst as *mut [std::mem::MaybeUninit<u8>] as *mut [u8]);
            let mut read_buf = tokio::io::ReadBuf::new(slice);
            match tokio::io::AsyncRead::poll_read(
                std::pin::Pin::new(&mut self.0),
                cx,
                &mut read_buf,
            ) {
                std::task::Poll::Ready(Ok(())) => read_buf.filled().len(),
                other => return other,
            }
        };
        unsafe { buf.advance(n) };
        std::task::Poll::Ready(Ok(()))
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> hyper2::rt::Write for TokioIo<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut self.0), cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut self.0), cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut self.0), cx)
    }
}

/// Send a single HTTP request over a new TLS+H2 connection.
///
/// This is the simplest transport — no connection pooling, no keep-alive.
/// Each call establishes a fresh connection, performs the H2 handshake,
/// and sends the request.
pub(crate) async fn send_request(
    connector: &FingerprintConnector,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>, String)> {
    let host = url.host_str().ok_or_else(|| Error::Config("no host in URL".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    // TLS connect.
    let tls_stream = connector
        .connect(host, port)
        .await
        .map_err(|e| Error::Tls(e.to_string()))?;

    let is_h2 = tls_stream
        .alpn
        .as_deref()
        .map_or(false, |a| a == b"h2");

    // Wrap for hyper2.
    let io = TokioIo(tls_stream.stream);

    if is_h2 {
        send_h2(io, h2_config, method, url, host, headers, body).await
    } else {
        // TODO: HTTP/1.1 fallback
        Err(Error::Http("HTTP/1.1 not yet implemented".into()))
    }
}

async fn send_h2<T>(
    io: T,
    h2_config: &H2Config,
    method: &str,
    url: &url::Url,
    host: &str,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>, String)>
where
    T: hyper2::rt::Read + hyper2::rt::Write + Unpin + Send + 'static,
{
    use h2::frame::{PseudoOrder, SettingsOrder};

    // Build H2 connection with fingerprint settings.
    let mut h2_builder = http2::Builder::new(TokioExecutor);

    // Apply H2 profile settings.
    for &(ref id, val) in &h2_config.settings {
        match *id {
            leyline_h2::SettingId::HeaderTableSize => { h2_builder.header_table_size(val); }
            leyline_h2::SettingId::EnablePush => { h2_builder.enable_push(val != 0); }
            leyline_h2::SettingId::MaxConcurrentStreams => { h2_builder.max_concurrent_streams(val); }
            leyline_h2::SettingId::InitialWindowSize => { h2_builder.initial_stream_window_size(val); }
            leyline_h2::SettingId::MaxFrameSize => { h2_builder.max_frame_size(val); }
            leyline_h2::SettingId::MaxHeaderListSize => { h2_builder.max_header_list_size(val); }
            _ => {}
        }
    }

    // Connection window size.
    h2_builder.initial_connection_window_size(h2_config.initial_connection_window_size);

    // Pseudo-header order.
    let pseudo_order = h2_config.pseudo_order.map(|p| match p {
        leyline_h2::PseudoOrder::Method => PseudoOrder::Method,
        leyline_h2::PseudoOrder::Authority => PseudoOrder::Authority,
        leyline_h2::PseudoOrder::Scheme => PseudoOrder::Scheme,
        leyline_h2::PseudoOrder::Path => PseudoOrder::Path,
    });
    h2_builder.headers_pseudo_order(Some(pseudo_order));

    // Settings order.
    let mut settings_arr = [SettingsOrder::HeaderTableSize; 8];
    for (i, id) in h2_config.settings_order.iter().enumerate().take(8) {
        settings_arr[i] = match *id {
            leyline_h2::SettingId::HeaderTableSize => SettingsOrder::HeaderTableSize,
            leyline_h2::SettingId::EnablePush => SettingsOrder::EnablePush,
            leyline_h2::SettingId::MaxConcurrentStreams => SettingsOrder::MaxConcurrentStreams,
            leyline_h2::SettingId::InitialWindowSize => SettingsOrder::InitialWindowSize,
            leyline_h2::SettingId::MaxFrameSize => SettingsOrder::MaxFrameSize,
            leyline_h2::SettingId::MaxHeaderListSize => SettingsOrder::MaxHeaderListSize,
            leyline_h2::SettingId::Unknown8 => SettingsOrder::UnknownSetting8,
            leyline_h2::SettingId::Unknown9 => SettingsOrder::UnknownSetting9,
        };
    }
    h2_builder.settings_order(Some(settings_arr));

    // Handshake.
    let (mut send_req, conn) = h2_builder
        .handshake(io)
        .await
        .map_err(|e| Error::Http(e.to_string()))?;

    // Spawn connection driver.
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!(error = %e, "h2 connection closed");
        }
    });

    // Build HTTP request.
    // Use full URI so hyper2 generates correct :scheme and :authority pseudo-headers.
    let http_method = method.parse::<http::Method>()
        .map_err(|e| Error::Config(format!("invalid method: {e}")))?;

    let full_uri = url.as_str();

    let mut req_builder = http::Request::builder()
        .method(http_method)
        .uri(full_uri);

    // Add ordered headers.
    for (name, value) in &headers {
        req_builder = req_builder.header(name.as_str(), value.as_str());
    }

    let req_body = match body {
        Some(b) => Full::new(Bytes::from(b)),
        None => Full::new(Bytes::new()),
    };

    let req = req_builder
        .body(req_body)
        .map_err(|e| Error::Http(e.to_string()))?;

    // Send request.
    let resp = send_req
        .send_request(req)
        .await
        .map_err(|e| Error::Http(e.to_string()))?;

    let status = resp.status().as_u16();
    let final_url = url.to_string();

    // Collect response headers.
    let resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Collect body.
    let body_bytes = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| Error::Http(e.to_string()))?
        .to_bytes()
        .to_vec();

    Ok((status, resp_headers, body_bytes, final_url))
}
