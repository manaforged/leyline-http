use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::h2::H2Config;
use crate::h2::H2Error;
use crate::h2::codec::{FrameReader, FrameWriter};
use crate::h2::connection::{Flood, FloodGuard, PeerSettings};
use crate::h2::frame::DEFAULT_HEADER_TABLE_SIZE;
use crate::h2::hpack;
use crate::profile::{DEFAULT_WINDOW_SIZE, SettingId};

use super::super::handle::{H2Client, PRE_SETTINGS_STREAM_CAPACITY, Shared};

use super::*;

pub async fn start<T>(io: T, config: H2Config) -> Result<H2Client, H2Error>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut io = io;
    let mut reader = FrameReader::new();
    let mut writer = FrameWriter::new();

    writer.write_preface();

    let settings_frame = SettingsFrame {
        ack: false,
        params: config
            .settings
            .iter()
            .map(|&(id, val)| (id.code(), val))
            .collect(),
    };
    settings_frame.encode(writer.out());

    if config.initial_connection_window_size > DEFAULT_WINDOW_SIZE {
        let increment = config.initial_connection_window_size - DEFAULT_WINDOW_SIZE;
        WindowUpdateFrame {
            stream_id: 0,
            increment,
        }
        .encode(writer.out());
    }
    writer.flush(&mut io).await?;

    let peer_settings = PeerSettings::default();
    let initial_send_window = i64::from(DEFAULT_WINDOW_SIZE);

    if let Some(ours) = config.setting(SettingId::MaxFrameSize) {
        reader.set_max_frame_size(ours);
    }

    let encoder = hpack::Encoder::new();
    let mut decoder = hpack::Decoder::new();
    let our_header_table_size = config
        .setting(SettingId::HeaderTableSize)
        .unwrap_or(DEFAULT_HEADER_TABLE_SIZE) as usize;
    decoder.set_max_header_list_size(config.header_list_limit());
    decoder.set_max_table_size(our_header_table_size);

    let (tx, rx) = super::protocol::command_channel(COMMAND_CHANNEL_CAPACITY);
    let (ping_tx, ping_rx) = mpsc::channel(PING_CHANNEL_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));
    let unanswered_timeout = Arc::new(AtomicBool::new(false));
    let streams = crate::h2::client::handle::Streams::new(PRE_SETTINGS_STREAM_CAPACITY);

    let driver = Driver {
        io,
        reader,
        writer,
        encoder,
        decoder,
        peer_settings,
        peer_greeted: false,
        local_settings_acked: false,
        conn_send_window: initial_send_window,
        conn_recv_window: config.initial_connection_window_size as i64,
        streams: super::stream_map::StreamMap::new(Arc::clone(&streams)),
        next_stream_id: 1,
        rst_flood: FloodGuard::new(
            Flood::RstStream,
            config.limits().rst_stream_flood_threshold,
            config.limits().rst_stream_flood_window,
        ),
        settings_flood: FloodGuard::new(
            Flood::Settings,
            config.limits().settings_flood_threshold,
            config.limits().settings_flood_window,
        ),
        control_flood: FloodGuard::new(
            Flood::Control,
            config.limits().control_flood_threshold,
            config.limits().control_flood_window,
        ),
        config: config.clone(),
        command_rx: rx,
        ping_rx,
        ping_armed: false,
        closed: closed.clone(),
        unanswered_timeout: Arc::clone(&unanswered_timeout),
        peer_goaway_last_stream: None,
        pending: VecDeque::new(),
        shutdown_started: false,
        body_chunks: None,
        ping_seq: 0,
        pings: VecDeque::new(),
        output: output::OutputState::default(),
    };

    drop(tokio::spawn(async move {
        if let Err(error) = driver.run().await {
            tracing::debug!(target: "leyline::h2", %error, "h2 connection driver stopped");
        }
    }));

    Ok(H2Client {
        shared: Arc::new(Shared {
            tx,
            ping_tx,
            closed,
            unanswered_timeout,
            streams,
        }),
    })
}
