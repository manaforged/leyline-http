use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;
use tokio::sync::{mpsc, oneshot};

use crate::error::BodyLimit;
use crate::profile::PseudoOrder;
use crate::quic::connection::{EstablishedH3, close_reason, flush_egress};
use crate::wire::BoxedStream;
use crate::wire::ResponseMode;
use crate::wire::{Chunk, Failure, TransportBody, TransportResponse, Upload};
use crate::wire::{ResponseBlock, ResponseState};

const STREAM_PUMP_INTERVAL: Duration = Duration::from_millis(2);

const CANCEL_SWEEP_INTERVAL: Duration = Duration::from_millis(100);

const LENGTH_MISMATCH: &str = "h3: response body length does not match content-length";

type H3BodyChunk = crate::util::upload::BodyChunk<u64>;

type StreamBody = (mpsc::Sender<Chunk>, mpsc::OwnedPermit<Chunk>);

enum H3Command {
    Request {
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        body_stream: Option<BoxedStream>,
        stream_body: Option<StreamBody>,
        mode: ResponseMode,
        head_request: bool,
        body_limit: BodyLimit,
        resp_tx: oneshot::Sender<Result<TransportResponse, H3SendError>>,
    },
}

#[derive(Debug)]
pub enum H3SendError {
    NotSent(String),
    Rejected(String),
    Closed(String),
    Failed(String),
    BodyLimit(BodyLimit),
    RequestBody(std::io::Error),
}

impl H3SendError {
    pub(crate) fn message(&self) -> Cow<'_, str> {
        match self {
            H3SendError::NotSent(m)
            | H3SendError::Rejected(m)
            | H3SendError::Closed(m)
            | H3SendError::Failed(m) => Cow::Borrowed(m.as_str()),
            H3SendError::BodyLimit(limit) => Cow::Owned(limit.to_string()),
            H3SendError::RequestBody(error) => {
                Cow::Owned(format!("request body stream failed: {error}"))
            }
        }
    }

    pub(crate) fn failure(&self) -> Failure {
        match self {
            H3SendError::NotSent(_) => Failure::Unsent,
            H3SendError::Rejected(_) => Failure::Refused,
            H3SendError::Closed(_) => Failure::Connection,
            H3SendError::Failed(_) | H3SendError::BodyLimit(_) | H3SendError::RequestBody(_) => {
                Failure::Request
            }
        }
    }
}

impl std::fmt::Display for H3SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for H3SendError {}

#[derive(Clone)]
pub struct H3Client {
    tx: mpsc::Sender<H3Command>,
    closed: Arc<AtomicBool>,
    open_streams: Arc<AtomicUsize>,
    pseudo_order: [PseudoOrder; 4],
}

impl H3Client {
    /// Returns whether both handles drive the same connection.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.closed, &other.closed)
    }

    pub fn open_streams(&self) -> usize {
        self.open_streams.load(Ordering::Relaxed)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub async fn send_request(
        &self,
        method: &http::Method,
        authority: &str,
        path: &str,
        headers: &[crate::wire::HeaderPair],
        body: Option<Bytes>,
        body_stream: Option<BoxedStream>,
        mode: ResponseMode,
        body_limit: BodyLimit,
    ) -> Result<TransportResponse, H3SendError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H3SendError::NotSent("h3 connection closed".into()));
        }
        let (stream_body, stream_body_rx) = match mode {
            ResponseMode::Buffered => (None, None),
            ResponseMode::Streamed | ResponseMode::ErrorPrefix(_) => {
                let (tx, terminal, rx) = crate::wire::body_channel();
                (Some((tx, terminal)), Some(rx))
            }
        };
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(H3Command::Request {
                headers: self.field_list(method, authority, path, headers),
                body,
                body_stream,
                stream_body,
                mode,
                head_request: *method == http::Method::HEAD,
                body_limit,
                resp_tx,
            })
            .await
            .map_err(|e| H3SendError::NotSent(format!("h3 driver task has exited: {e}")))?;
        match resp_rx.await {
            Ok(result) => result.map(|head| head.streamed(mode, stream_body_rx)),
            Err(_) => Err(H3SendError::Closed(
                "h3 driver dropped response sender".into(),
            )),
        }
    }

    fn field_list(
        &self,
        method: &http::Method,
        authority: &str,
        path: &str,
        headers: &[crate::wire::HeaderPair],
    ) -> Vec<quiche::h3::Header> {
        let uri_path = if path.is_empty() { "/" } else { path };
        let mut list = Vec::with_capacity(4 + headers.len());
        for pseudo in self.pseudo_order {
            let (name, value): (&[u8], &[u8]) = match pseudo {
                PseudoOrder::Method => (b":method", method.as_str().as_bytes()),
                PseudoOrder::Scheme => (b":scheme", b"https"),
                PseudoOrder::Authority => (b":authority", authority.as_bytes()),
                PseudoOrder::Path => (b":path", uri_path.as_bytes()),
            };
            list.push(quiche::h3::Header::new(name, value));
        }
        for (k, v) in headers {
            list.push(quiche::h3::Header::new(k.as_bytes(), v.as_bytes()));
        }
        list
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

struct H3Stream {
    resp_tx: Option<oneshot::Sender<Result<TransportResponse, H3SendError>>>,
    response: ResponseState,
    status: http::StatusCode,
    headers: http::HeaderMap,
    trailers: http::HeaderMap,
    body: Vec<u8>,
    stream_tx: Option<mpsc::Sender<Chunk>>,
    terminal: Option<mpsc::OwnedPermit<Chunk>>,
    mode: ResponseMode,
    head_sent: bool,
    stalled: Option<Bytes>,
    peer_finished: bool,
    body_bytes_seen: usize,
    declared_len: Option<u64>,
    head_request: bool,
    body_limit: BodyLimit,
    out_chunks: VecDeque<Bytes>,
    out_offset: usize,
    body_eof: bool,
    fin_sent: bool,
    upload: Option<Upload>,
}

struct H3Driver {
    established: EstablishedH3,
    command_rx: mpsc::Receiver<H3Command>,
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    open_streams: Arc<AtomicUsize>,
    streams: HashMap<u64, H3Stream>,
}

mod driver;
mod open;
pub(crate) use open::open_fresh_h3;
mod response;
use response::*;
mod stream;

#[cfg(test)]
mod tests;
