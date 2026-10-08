#![allow(
    dead_code,
    reason = "shared by several test binaries; each binary uses a subset"
)]

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::frame::{
    DataFrame, FRAME_HEADER_LEN, FrameHeader, FrameType, HeadersFrame, SettingsFrame,
    WindowUpdateFrame,
};
use leyline::h2::{H2Client, Head, RequestBody, hpack};

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const END_HEADERS: u8 = 0x4;

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

pub fn test_config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65535),
            (SettingId::MaxFrameSize, 16384),
        ],
        settings_order: vec![
            SettingId::HeaderTableSize,
            SettingId::EnablePush,
            SettingId::InitialWindowSize,
            SettingId::MaxFrameSize,
        ],
        pseudo_order: [
            PseudoOrder::Method,
            PseudoOrder::Authority,
            PseudoOrder::Scheme,
            PseudoOrder::Path,
        ],
        initial_connection_window_size: 65535,
        default_priority: None,
        rst_stream_flood_threshold: 100,
        rst_stream_flood_window: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 100,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

pub fn get_head(path: &str) -> Head {
    Head {
        pseudo: PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "example.com".into(),
            path: path.into(),
            protocol: None,
        },
        headers: vec![("user-agent".into(), "test".into())],
    }
}

pub async fn write_end_headers<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32) {
    let mut frame = [0u8; FRAME_HEADER_LEN];
    frame[3] = FrameType::Continuation as u8;
    frame[4] = END_HEADERS;
    frame[5..].copy_from_slice(&stream_id.to_be_bytes());
    s.write_all(&frame).await.expect("continuation write");
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

pub const CHUNK: usize = 16 * 1024;
pub static ZEROS: [u8; CHUNK] = [0; CHUNK];

pub fn head(method: &str) -> Arc<Head> {
    Arc::new(Head {
        pseudo: PseudoHeaders {
            method: method.into(),
            scheme: "https".into(),
            authority: "example.com".into(),
            path: "/".into(),
            protocol: None,
        },
        headers: vec![],
    })
}

pub fn send(handle: &H2Client, method: &str, body: RequestBody) -> tokio::task::JoinHandle<()> {
    let handle = handle.clone();
    let head = head(method);
    tokio::spawn(async move {
        drop(handle.send_shared(head, body, false).await);
    })
}

#[derive(Default)]
pub struct Producer {
    pub produced: AtomicUsize,
    pub dropped: AtomicBool,
}

pub struct Endless(Arc<Producer>);

impl Stream for Endless {
    type Item = io::Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.produced.fetch_add(CHUNK, Ordering::SeqCst);
        Poll::Ready(Some(Ok(Bytes::from_static(&ZEROS))))
    }
}

impl Drop for Endless {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

pub fn endless(producer: &Arc<Producer>) -> RequestBody {
    RequestBody::Streaming {
        stream: Box::pin(Endless(Arc::clone(producer))),
        length_hint: None,
    }
}
