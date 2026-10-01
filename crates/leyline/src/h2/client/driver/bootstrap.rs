use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::config::H2Config;
use crate::h2::connection::{
    CONTROL_FLOOD_THRESHOLD, CONTROL_FLOOD_WINDOW, PeerSettings, RstFloodDetector, id_to_u16,
};
use crate::h2::error::H2Error;
use crate::h2::hpack;

use super::super::handle::H2Client;

use super::*;

pub async fn start<T>(io: T, config: H2Config) -> Result<H2Client, H2Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(io);
    let mut reader = FrameReader::new(read_half);
    let mut writer = FrameWriter::new(write_half);

    writer.write_preface().await?;

    let settings_frame = SettingsFrame {
        ack: false,
        params: config
            .settings
            .iter()
            .map(|(id, val)| (id_to_u16(id), *val))
            .collect(),
    };
    writer.write_settings(&settings_frame).await?;

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

    let peer_settings = PeerSettings::default();
    let initial_send_window: i64 = 65535;

    if let Some((_, ours)) = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::MaxFrameSize))
    {
        reader.set_max_frame_size(*ours);
    }

    let encoder = hpack::Encoder::new();
    let mut decoder = hpack::Decoder::new();
    let max_hdr = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::MaxHeaderListSize))
        .map(|(_, v)| *v as usize)
        .unwrap_or(crate::core::DEFAULT_MAX_HEADER_LIST_BYTES);
    let our_header_table_size = config
        .settings
        .iter()
        .find(|(id, _)| matches!(id, crate::h2::config::SettingId::HeaderTableSize))
        .map(|(_, v)| *v as usize)
        .unwrap_or(4096);
    decoder.set_max_header_list_size(max_hdr);
    decoder.set_max_table_size(our_header_table_size);

    let snapshot = Arc::new(PeerSettingsSnapshot::new());
    snapshot.set_max_concurrent_streams(peer_settings.max_concurrent_streams);
    snapshot.set_enable_connect_protocol(peer_settings.enable_connect_protocol);

    let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (ping_tx, ping_rx) = mpsc::channel(PING_CHANNEL_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));

    let (body_chunk_tx, body_chunk_rx) = mpsc::channel(STREAM_REQ_BODY_CAPACITY * 4);

    let driver = Driver {
        reader,
        writer,
        encoder,
        decoder,
        peer_settings,
        peer_greeted: false,
        peer_snapshot: snapshot.clone(),
        conn_send_window: initial_send_window,
        conn_recv_window: config.initial_connection_window_size as i64,
        streams: super::stream_map::StreamMap::new(),
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
        control_flood: RstFloodDetector::with_label(
            CONTROL_FLOOD_THRESHOLD,
            CONTROL_FLOOD_WINDOW,
            "leyline::h2::control_flood",
            "peer sent excessive PING, WINDOW_UPDATE or empty CONTINUATION frames",
        ),
        config: config.clone(),
        command_rx: rx,
        ping_rx,
        closed: closed.clone(),
        peer_goaway_last_stream: None,
        pending: VecDeque::new(),
        shutdown_started: false,
        body_chunk_tx,
        body_chunk_rx,
        ping_seq: 0,
        pings: VecDeque::new(),
        stalled: 0,
    };

    drop(tokio::spawn(driver.run()));

    Ok(H2Client {
        tx,
        ping_tx,
        closed,
        #[cfg(feature = "websocket")]
        peer_settings: snapshot,
    })
}
