#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::BytesMut;
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{AlpnError, Ssl, SslContextBuilder, SslMethod, select_next_proto};
use leyline_bssl::x509::X509;
use leyline_bssl_tokio::SslStream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use leyline::h2::frame::{
    DataFrame, FRAME_HEADER_LEN, FrameHeader, HeadersFrame, SettingsFrame, WindowUpdateFrame,
};
use leyline::h2::hpack;

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub async fn read_preface<S: AsyncRead + Unpin>(s: &mut S) {
    let mut buf = [0u8; 24];
    s.read_exact(&mut buf).await.expect("preface read");
    assert_eq!(&buf[..], PREFACE, "bad preface bytes");
}

pub async fn read_frame<S: AsyncRead + Unpin>(s: &mut S) -> (FrameHeader, Vec<u8>) {
    let mut hdr_buf = [0u8; FRAME_HEADER_LEN];
    s.read_exact(&mut hdr_buf).await.expect("frame header read");
    let hdr = FrameHeader::parse(&hdr_buf);
    let mut payload = vec![0u8; hdr.length as usize];
    if hdr.length > 0 {
        s.read_exact(&mut payload)
            .await
            .expect("frame payload read");
    }
    (hdr, payload)
}

pub async fn write_server_settings<S: AsyncWrite + Unpin>(s: &mut S) {
    let frame = SettingsFrame {
        ack: false,
        params: vec![],
    };
    let mut buf = BytesMut::new();
    frame.encode(&mut buf);
    s.write_all(&buf).await.expect("server settings write");
}

pub async fn write_server_settings_with<S: AsyncWrite + Unpin>(s: &mut S, params: Vec<(u16, u32)>) {
    let frame = SettingsFrame { ack: false, params };
    let mut buf = BytesMut::new();
    frame.encode(&mut buf);
    s.write_all(&buf).await.expect("server settings write");
}

pub async fn write_settings_ack<S: AsyncWrite + Unpin>(s: &mut S) {
    let frame = SettingsFrame::ack();
    let mut buf = BytesMut::new();
    frame.encode(&mut buf);
    s.write_all(&buf).await.expect("settings ack write");
}

pub async fn write_response<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32, body: &[u8]) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(&[(":status", "200")]);
    let h = HeadersFrame {
        stream_id,
        end_stream: false,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("resp headers write");

    let d = DataFrame {
        stream_id,
        end_stream: true,
        data: bytes::Bytes::copy_from_slice(body),
        wire_len: body.len() as u64,
    };
    buf.clear();
    d.encode(&mut buf);
    s.write_all(&buf).await.expect("resp data write");
}

pub async fn write_raw_headers<S: AsyncWrite + Unpin>(
    s: &mut S,
    stream_id: u32,
    headers: &[(&str, &str)],
    end_stream: bool,
) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(headers);
    let h = HeadersFrame {
        stream_id,
        end_stream,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("raw headers write");
}

pub async fn write_headers_without_end<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(&[(":status", "200")]);
    let h = HeadersFrame {
        stream_id,
        end_stream: false,
        end_headers: false,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("partial headers write");
}

pub async fn write_response_headers<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32) {
    let mut enc = hpack::Encoder::new();
    let fragment = enc.encode_header_block(&[(":status", "200")]);
    let h = HeadersFrame {
        stream_id,
        end_stream: false,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from(fragment),
    };
    let mut buf = BytesMut::new();
    h.encode(&mut buf);
    s.write_all(&buf).await.expect("resp headers write");
}

pub async fn write_data<S: AsyncWrite + Unpin>(
    s: &mut S,
    stream_id: u32,
    data: &[u8],
    end_stream: bool,
) {
    let d = DataFrame {
        stream_id,
        end_stream,
        data: bytes::Bytes::copy_from_slice(data),
        wire_len: data.len() as u64,
    };
    let mut buf = BytesMut::new();
    d.encode(&mut buf);
    s.write_all(&buf).await.expect("data write");
}

pub async fn write_window_update<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32, inc: u32) {
    let w = WindowUpdateFrame {
        stream_id,
        increment: inc,
    };
    let mut buf = BytesMut::new();
    w.encode(&mut buf);
    s.write_all(&buf).await.expect("window update write");
}

pub async fn tls_server<F, Fut>(
    cert: X509,
    key: PKey<Private>,
    connections: Arc<AtomicUsize>,
    serve: F,
) -> u16
where
    F: Fn(SslStream<TcpStream>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local address").port();
    let mut context = SslContextBuilder::new(SslMethod::tls()).expect("TLS context");
    context.set_certificate(&cert).expect("certificate");
    context.set_private_key(&key).expect("private key");
    context.set_alpn_select_callback(|_, offered| {
        select_next_proto(b"\x02h2", offered).ok_or(AlpnError::NOACK)
    });
    let context = context.build();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            connections.fetch_add(1, Ordering::SeqCst);
            let ssl = Ssl::new(&context).expect("TLS session");
            if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                .accept()
                .await
            {
                tokio::spawn(serve(stream));
            }
        }
    });
    port
}
