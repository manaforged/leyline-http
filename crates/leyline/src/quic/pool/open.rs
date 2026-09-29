use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::mpsc;

use super::{H3Client, H3Driver};
use crate::pool::TlsInfo;
use crate::quic::config::H3Config;
use crate::quic::connection::connect_and_handshake;
use crate::tls::{FingerprintConnector, TlsError, TlsTrustConfig};
use crate::{Error, Kind};

const COMMAND_CHANNEL_CAPACITY: usize = 1024;

const STREAM_REQ_CAPACITY: usize = 64;

pub(crate) async fn open_fresh_h3(
    h3_cfg: &H3Config,
    trust: &TlsTrustConfig,
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: Option<&str>,
) -> Result<(H3Client, TlsInfo), Error> {
    let handshake = connect_and_handshake(h3_cfg, trust, connector, host, port, proxy);
    let established = connector
        .with_timeout(async { Ok::<_, TlsError>(handshake.await) })
        .await
        .map_err(|timeout| {
            Error::from(timeout)
                .with_message(format!("h3 handshake to {host}:{port}: connect timeout"))
        })?
        .map_err(|message| Error::new(Kind::Http3).with_message(message))?;
    let tls = established.tls.clone();

    let (tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
    let (body_chunk_tx, body_chunk_rx) = mpsc::channel(STREAM_REQ_CAPACITY);
    let closed = Arc::new(AtomicBool::new(false));

    let driver = H3Driver {
        established,
        command_rx,
        body_chunk_tx,
        body_chunk_rx,
        closed: Arc::clone(&closed),
        streams: HashMap::new(),
    };
    drop(tokio::spawn(driver.run()));

    Ok((
        H3Client {
            tx,
            closed,
            pseudo_order: h3_cfg.pseudo_order,
        },
        tls,
    ))
}
