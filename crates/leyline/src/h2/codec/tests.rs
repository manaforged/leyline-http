use super::*;
use crate::h2::H2Error;
use crate::h2::frame::{Frame, FrameHeader, SettingsFrame};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn small_frames_do_not_reserve_a_tls_record() {
    let reader = FrameReader::new();
    assert!(reader.read_capacity() < 4 * 1024);
    assert!(reader.read_capacity() <= super::reader::SLACK);
}

#[tokio::test]
async fn write_and_read_settings() {
    let (client, server) = tokio::io::duplex(4096);

    let frame = SettingsFrame {
        ack: false,
        params: vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)],
    };

    let mut client = client;
    let mut writer = FrameWriter::new();
    frame.encode(writer.out());
    writer
        .flush(&mut client)
        .await
        .expect("SETTINGS bytes reach the peer");
    drop(client);

    let mut server = server;
    let mut reader = FrameReader::new();
    let read_frame = reader.next(&mut server).await.unwrap().unwrap();

    match read_frame {
        Frame::Settings(s) => {
            assert!(!s.ack);
            assert_eq!(s.params, vec![(0x1, 65536), (0x2, 0), (0x4, 6291456)]);
        }
        _ => panic!("expected Settings frame"),
    }

    assert!(reader.next(&mut server).await.unwrap().is_none());
}

#[tokio::test]
async fn write_and_read_preface_then_settings() {
    let (client, server) = tokio::io::duplex(4096);

    let mut client = client;
    let mut writer = FrameWriter::new();
    writer.write_preface();
    SettingsFrame {
        ack: false,
        params: vec![(0x4, 6291456)],
    }
    .encode(writer.out());
    writer
        .flush(&mut client)
        .await
        .expect("preface and SETTINGS reach the peer");
    drop(client);

    let mut reader_raw = server;
    let mut preface = [0u8; 24];
    reader_raw.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");

    let mut reader = FrameReader::new();
    let frame = reader.next(&mut reader_raw).await.unwrap().unwrap();
    assert!(matches!(frame, Frame::Settings(_)));
}

#[tokio::test]
async fn next_resumes_after_cancellation() {
    let (client, mut server) = tokio::io::duplex(1024);

    let frame = SettingsFrame {
        ack: false,
        params: vec![(0x1, 65536), (0x4, 6291456)],
    };
    let mut encoded = BytesMut::new();
    frame.encode(&mut encoded);
    let encoded = encoded.freeze();

    let server_task = tokio::spawn(async move {
        for byte in encoded.iter() {
            server.write_all(&[*byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        drop(server);
    });

    let mut client = client;
    let mut reader = FrameReader::new();
    let read_frame = loop {
        tokio::select! {
            biased;
            res = reader.next(&mut client) => {
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

    let mut server = server;
    let mut reader = FrameReader::new();
    let result = reader.next(&mut server).await;
    assert!(matches!(result, Err(H2Error::FrameTooLarge { .. })));
}
