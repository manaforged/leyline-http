use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::config::{DEFAULT_HEADER_TABLE_SIZE, DEFAULT_WINDOW_SIZE, H2Config, SettingId};
use crate::h2::connection::{
    CONTROL_FLOOD_THRESHOLD, CONTROL_FLOOD_WINDOW, PeerSettings, RstFloodDetector,
};
use crate::h2::error::H2Error;
use crate::h2::hpack;

use super::super::handle::H2Client;

use super::*;

const INITIAL_MAX_CONCURRENT_STREAMS: usize = 100;

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
            .map(|&(id, val)| (id as u16, val))
            .collect(),
    };
    writer.write_settings(&settings_frame).await?;

    if config.initial_connection_window_size > DEFAULT_WINDOW_SIZE {
        let increment = config.initial_connection_window_size - DEFAULT_WINDOW_SIZE;
        writer
            .write_frame(&WindowUpdateFrame {
                stream_id: 0,
                increment,
            })
            .await?;
    }
    writer.flush().await?;

    let peer_settings = PeerSettings::default();
    let initial_send_window = i64::from(DEFAULT_WINDOW_SIZE);

    if let Some(ours) = config.setting(SettingId::MaxFrameSize) {
        reader.set_max_frame_size(ours);
    }

    let encoder = hpack::Encoder::new();
    let mut decoder = hpack::Decoder::new();
    let max_hdr = config
        .setting(SettingId::MaxHeaderListSize)
        .map_or(crate::core::DEFAULT_MAX_HEADER_LIST_BYTES, |v| v as usize);
    let our_header_table_size = config
        .setting(SettingId::HeaderTableSize)
        .unwrap_or(DEFAULT_HEADER_TABLE_SIZE) as usize;
    decoder.set_max_header_list_size(max_hdr);
    decoder.set_max_table_size(our_header_table_size);

    let snapshot = Arc::new(PeerSettingsSnapshot::new());
    snapshot.set_enable_connect_protocol(peer_settings.enable_connect_protocol);

    let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (ping_tx, ping_rx) = mpsc::channel(PING_CHANNEL_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));
    let unanswered_timeout = Arc::new(AtomicBool::new(false));
    let streams = crate::h2::client::handle::Streams::new(INITIAL_MAX_CONCURRENT_STREAMS);

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
        streams: super::stream_map::StreamMap::new(Arc::clone(&streams)),
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
        unanswered_timeout: Arc::clone(&unanswered_timeout),
        load: Arc::clone(&streams),
        peer_goaway_last_stream: None,
        pending: VecDeque::new(),
        shutdown_started: false,
        body_chunk_tx,
        body_chunk_rx,
        ping_seq: 0,
        pings: VecDeque::new(),
        stalled: 0,
        output: output::OutputState::default(),
    };

    drop(tokio::spawn(driver.run()));

    Ok(H2Client {
        tx,
        ping_tx,
        closed,
        unanswered_timeout,
        streams,
        #[cfg(feature = "websocket")]
        peer_settings: snapshot,
    })
}
