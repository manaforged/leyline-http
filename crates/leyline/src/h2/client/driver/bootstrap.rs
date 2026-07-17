//! Connection bootstrap: handshake and driver task spawn.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::config::H2Config;
use crate::h2::connection::{PeerSettings, RstFloodDetector, id_to_u16};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::hpack;

use super::super::handle::H2Client;

use super::*;

/// Pump a user `Stream<Item = io::Result<Bytes>>` into an mpsc the driver
/// can `recv` from. Runs in its own task so the driver's single-task
/// invariant is preserved.
pub(crate) async fn pump_request_body(
    mut stream: Pin<Box<dyn futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static>>,
    tx: mpsc::Sender<io::Result<Bytes>>,
) {
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let is_err = chunk.is_err();
        if tx.send(chunk).await.is_err() {
            // Receiver dropped — driver failed or already finished.
            return;
        }
        if is_err {
            return;
        }
    }
    // Drop tx to signal EOF.
}

/// Per-stream relay: forwards chunks from the caller-owned `rx` into
/// the driver-wide `chunk_tx`, tagging each message with `stream_id`.
pub(super) async fn relay_request_body(
    stream_id: u32,
    mut rx: mpsc::Receiver<io::Result<Bytes>>,
    chunk_tx: mpsc::Sender<BodyChunkIn>,
) {
    while let Some(item) = rx.recv().await {
        match item {
            Ok(data) => {
                if data.is_empty() {
                    continue;
                }
                if chunk_tx
                    .send(BodyChunkIn::Chunk { stream_id, data })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(e) => {
                let _ = chunk_tx
                    .send(BodyChunkIn::Eof {
                        stream_id,
                        error: Some(e),
                    })
                    .await;
                return;
            }
        }
    }
    let _ = chunk_tx
        .send(BodyChunkIn::Eof {
            stream_id,
            error: None,
        })
        .await;
}

/// Spawn a driver task over the given IO, after performing the HTTP/2
/// handshake (preface + SETTINGS exchange).
pub(crate) async fn start<T>(io: T, config: H2Config) -> Result<(H2Client, DriverTask), H2Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(io);
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    // 1. Preface.
    writer.write_preface().await?;

    // 2. Our SETTINGS (ordered per fingerprint config).
    let settings_frame = SettingsFrame {
        ack: false,
        params: config
            .settings
            .iter()
            .map(|(id, val)| (id_to_u16(id), *val))
            .collect(),
    };
    writer.write_settings(&settings_frame).await?;

    // 3. WINDOW_UPDATE if connection window > default 65535.
    let default_window: u32 = 65535;
    if config.initial_connection_window_size > default_window {
        let increment = config.initial_connection_window_size - default_window;
        writer
            .write_window_update(&WindowUpdateFrame {
                stream_id: 0,
                increment,
            })
            .await?;
    }
    writer.flush().await?;

    // 4. Read the server's SETTINGS. Some real servers delay or omit the
    //    ACK for our initial SETTINGS until after request traffic starts;
    //    browser clients do not block request dispatch on that ACK. The
    //    driver still accepts a later ACK once it is running.
    let mut peer_settings = PeerSettings::default();
    let mut got_settings = false;
    let mut initial_send_window: i64 = 65535;
    let deadline = tokio::time::Instant::now() + config.settings_ack_timeout;

    while !got_settings {
        let next = match tokio::time::timeout_at(deadline, reader.next()).await {
            Ok(inner) => inner?,
            Err(_) => {
                tracing::warn!(
                    target: "leyline::h2::handshake",
                    timeout_ms = config.settings_ack_timeout.as_millis() as u64,
                    got_peer_settings = got_settings,
                    "SETTINGS_TIMEOUT — peer did not send SETTINGS; tearing down connection"
                );
                return Err(H2Error::Connection {
                    code: ErrorCode::SettingsTimeout,
                    reason: format!(
                        "peer did not send SETTINGS within {:?}",
                        config.settings_ack_timeout
                    ),
                });
            }
        };
        let frame = next.ok_or_else(|| H2Error::Connection {
            code: ErrorCode::ProtocolError,
            reason: "connection closed before SETTINGS".into(),
        })?;

        match frame {
            Frame::Settings(s) if !s.ack => {
                let _result = peer_settings.apply(&s.params)?;
                writer.write_settings_ack().await?;
                writer.flush().await?;
                got_settings = true;
            }
            Frame::Settings(_s) if _s.ack => {}
            Frame::WindowUpdate(w) if w.stream_id == 0 => {
                // §6.9.1 overflow check during handshake as well;
                // a malicious peer sending WU(0, 2^31-1) twice must
                // be rejected before we start writing DATA frames.
                initial_send_window = match checked_window_add(
                    initial_send_window,
                    w.increment as i64,
                ) {
                    Ok(v) => v,
                    Err(new_win) => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::FlowControlError,
                            reason: format!(
                                "handshake WINDOW_UPDATE would push connection window to {new_win} (> 2^31-1)"
                            ),
                        });
                    }
                };
            }
            Frame::WindowUpdate(_) => {}
            Frame::GoAway(g) => {
                return Err(H2Error::Connection {
                    code: g.error_code,
                    reason: format!("server sent GOAWAY during handshake: {:?}", g.error_code),
                });
            }
            _ => {}
        }
    }

    // Cap inbound frames at *our* advertised SETTINGS_MAX_FRAME_SIZE, never the
    // peer's. RFC 9113 §4.2: a SETTINGS parameter bounds the frames its *sender*
    // will receive — so `peer_settings.max_frame_size` limits what *we send*
    // (the writer honours it), while what we *accept* is what we advertised. The
    // reader is constructed at the 16384 default, which is exactly right when we
    // advertise no MAX_FRAME_SIZE (Chrome/Safari/okhttp); only override it if a
    // profile explicitly advertises a larger value. Using the peer's value here
    // let a hostile server advertise a huge frame size and make us accept — and
    // pre-allocate — frames far larger than we ever agreed to receive.
    if let Some((_, ours)) = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::MaxFrameSize))
    {
        reader.set_max_frame_size(*ours);
    }

    // HPACK codecs.
    //
    // RFC 7541 §4.2 / RFC 9113 §6.5.2: SETTINGS_HEADER_TABLE_SIZE bounds the
    // peer's encoder, so *our* decoder's ceiling is what *we* advertised —
    // never what the peer advertised. The encoder's ceiling is set mid-
    // connection from peer SETTINGS.
    let encoder = hpack::Encoder::new();
    let mut decoder = hpack::Decoder::new();
    let max_hdr = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::MaxHeaderListSize))
        .map(|(_, v)| *v as usize)
        .unwrap_or(256 * 1024);
    let our_header_table_size = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::HeaderTableSize))
        .map(|(_, v)| *v as usize)
        .unwrap_or(4096);
    decoder.set_max_header_list_size(max_hdr);
    decoder.set_max_table_size(our_header_table_size);

    // Publish snapshot.
    let snapshot = Arc::new(PeerSettingsSnapshot::new());
    snapshot.set_max_concurrent_streams(peer_settings.max_concurrent_streams);
    snapshot.set_enable_connect_protocol(peer_settings.enable_connect_protocol);

    let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));

    let (body_chunk_tx, body_chunk_rx) = mpsc::channel(STREAM_REQ_BODY_CAPACITY * 4);

    let driver = Driver {
        reader,
        writer,
        encoder,
        decoder,
        peer_settings,
        peer_snapshot: snapshot.clone(),
        conn_send_window: initial_send_window,
        conn_recv_window: config.initial_connection_window_size as i64,
        streams: HashMap::new(),
        next_stream_id: 1,
        buffered_pending: VecDeque::new(),
        rst_flood: RstFloodDetector::new(
            config.rst_stream_flood_threshold,
            config.rst_stream_flood_window,
        ),
        settings_flood: RstFloodDetector::with_label(
            config.settings_flood_threshold,
            config.settings_flood_window,
            "leyline::h2::settings_flood",
            "peer sent excessive SETTINGS updates",
        ),
        config: config.clone(),
        command_rx: rx,
        closed: closed.clone(),
        peer_goaway_last_stream: None,
        shutdown_started: false,
        body_chunk_tx,
        body_chunk_rx,
    };

    let join = tokio::spawn(driver.run());

    Ok((
        H2Client {
            tx,
            closed,
            peer_settings: snapshot,
        },
        DriverTask { join },
    ))
}
