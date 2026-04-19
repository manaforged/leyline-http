//! Tiny mock HTTP/2 server helpers for driver-focused integration tests.
//!
//! Each test instantiates its own mock server inline; this module only
//! exposes a handful of frame-level read/write helpers that operate on a
//! `tokio::io::DuplexStream` (or any `AsyncRead + AsyncWrite`). We
//! deliberately avoid pulling in a mock framework — the tests are
//! exercising the driver's own frame handling, so the server side must
//! emit raw RFC 9113 bytes.

#![allow(dead_code)]

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use leyline::h2::frame::{
    DataFrame, FrameHeader, HeadersFrame, SettingsFrame, WindowUpdateFrame, FRAME_HEADER_LEN,
};
use leyline::h2::hpack;

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub async fn read_preface<S: AsyncRead + Unpin>(s: &mut S) {
    let mut buf = [0u8; 24];
    s.read_exact(&mut buf).await.expect("preface read");
    assert_eq!(&buf[..], PREFACE, "bad preface bytes");
}

/// Read a single frame header + payload from the wire.
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

/// Write an empty SETTINGS frame (used by the mock server to advertise
/// defaults) and its ACK to the given settings.
pub async fn write_server_settings<S: AsyncWrite + Unpin>(s: &mut S) {
    let frame = SettingsFrame {
        ack: false,
        params: vec![],
    };
    let mut buf = BytesMut::new();
    frame.encode(&mut buf);
    s.write_all(&buf).await.expect("server settings write");
}

/// Write a SETTINGS frame with arbitrary params.
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

/// Write a full response: HEADERS(:status) + DATA(body, END_STREAM).
pub async fn write_response<S: AsyncWrite + Unpin>(s: &mut S, stream_id: u32, body: &[u8]) {
    // Encode the header block with a fresh encoder per call — tests
    // don't rely on HPACK dynamic-table continuity across streams.
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
    };
    buf.clear();
    d.encode(&mut buf);
    s.write_all(&buf).await.expect("resp data write");
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
