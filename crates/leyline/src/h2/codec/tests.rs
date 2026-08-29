use super::*;
use crate::h2::H2Error;
use crate::h2::frame::{Frame, FrameHeader, SettingsFrame};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn write_and_read_settings() {
    let (client, server) = tokio::io::duplex(4096);

    let frame = SettingsFrame {
        ack: false,
        params: vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)],
    };

    let mut writer = FrameWriter::new(client);
    writer.write_settings(&frame).await.unwrap();
    drop(writer);

    let mut reader = FrameReader::new(server);
    let read_frame = reader.next().await.unwrap().unwrap();

    match read_frame {
        Frame::Settings(s) => {
            assert!(!s.ack);
            assert_eq!(s.params, vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)]);
        }
        _ => panic!("expected Settings frame"),
    }

    assert!(reader.next().await.unwrap().is_none());
}

#[tokio::test]
async fn write_and_read_preface_then_settings() {
    let (client, server) = tokio::io::duplex(4096);

    let mut writer = FrameWriter::new(client);
    writer.write_preface().await.unwrap();
    writer
        .write_settings(&SettingsFrame {
            ack: false,
            params: vec![(0x4, 6291456)],
        })
        .await
        .unwrap();
    drop(writer);

    let mut reader_raw = server;
    let mut preface = [0u8; 24];
    reader_raw.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");

    let mut reader = FrameReader::new(reader_raw);
    let frame = reader.next().await.unwrap().unwrap();
    assert!(matches!(frame, Frame::Settings(_)));
}

#[tokio::test]
async fn next_resumes_after_cancellation() {
    // Simulates the actor-model driver's `tokio::select!`: a sibling
    // branch wins mid-read, the reader future is dropped with only
    // part of a frame on the wire, and the next `next()` call must
    // resume instead of re-reading from the socket and desyncing.
    let (client, mut server) = tokio::io::duplex(1024);

    let frame = SettingsFrame {
        ack: false,
        params: vec![(0x1, 65536), (0x4, 6291456)],
    };
    let mut encoded = BytesMut::new();
    frame.encode(&mut encoded);
    let encoded = encoded.freeze();

    // Server writes one byte at a time so the reader's `read` call
    // will often return Pending in-between, giving the select! race
    // real opportunity to drop the reader future mid-header.
    let server_task = tokio::spawn(async move {
        for byte in encoded.iter() {
            server.write_all(&[*byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        drop(server);
    });

    let mut reader = FrameReader::new(client);
    let read_frame = loop {
        tokio::select! {
            biased;
            res = reader.next() => {
                break res.unwrap().unwrap();
            }
            _ = tokio::task::yield_now() => {
                continue;
            }
        }
    };

    server_task.await.unwrap();
    match read_frame {
        Frame::Settings(s) => {
            assert_eq!(s.params, vec![(0x1, 65536), (0x4, 6291456)]);
        }
        _ => panic!("expected Settings frame"),
    }
}

#[tokio::test]
async fn rejects_oversized_frame() {
    let (client, server) = tokio::io::duplex(4096);

    let mut writer = client;
    let header = FrameHeader {
        length: 32768,
        frame_type: 0x0,
        flags: 0,
        stream_id: 1,
    };
    let mut buf = BytesMut::with_capacity(9);
    header.encode(&mut buf);
    writer.write_all(&buf).await.unwrap();
    drop(writer);

    let mut reader = FrameReader::new(server);
    let result = reader.next().await;
    assert!(matches!(result, Err(H2Error::FrameTooLarge { .. })));
}
