use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::core::ResponseMode;
use crate::core::session::decompress::BodyLimit;
use crate::h2::config::PseudoOrder;
use crate::quic::connection::{
    EstablishedH3, H3Response, check_body_budget, close_reason, flush_egress,
};

const STREAM_RESP_CAPACITY: usize = 32;

const STREAM_PUMP_INTERVAL: Duration = Duration::from_millis(2);

const CANCEL_SWEEP_INTERVAL: Duration = Duration::from_millis(100);

const LENGTH_MISMATCH: &str = "h3: response body length does not match content-length";

pub type H3RequestBodyStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

type H3BodyChunk = crate::util::upload::BodyChunk<u64>;

enum H3Command {
    Request {
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        body_stream: Option<H3RequestBodyStream>,
        stream_body_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        mode: ResponseMode,
        retried: bool,
        resp_tx: oneshot::Sender<Result<H3Response, H3SendError>>,
    },
}

pub enum H3RespBody {
    Buffered(Vec<u8>),
    Streaming(mpsc::Receiver<std::io::Result<Bytes>>),
}

pub struct H3ResponseParts {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub trailers: Vec<(String, String)>,
    pub body: H3RespBody,
}

#[derive(Debug)]
pub enum H3SendError {
    NotSent(String),
    Rejected(String),
    Failed(String),
    BodyLimit(BodyLimit),
    RequestBody(std::io::Error),
}

impl H3SendError {
    pub(crate) fn message(&self) -> Cow<'_, str> {
        match self {
            H3SendError::NotSent(m) | H3SendError::Rejected(m) | H3SendError::Failed(m) => {
                Cow::Borrowed(m.as_str())
            }
            H3SendError::BodyLimit(limit) => Cow::Owned(limit.to_string()),
            H3SendError::RequestBody(error) => {
                Cow::Owned(format!("request body stream failed: {error}"))
            }
        }
    }

    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, H3SendError::NotSent(_))
    }
}

#[derive(Clone)]
pub struct H3Client {
    tx: mpsc::Sender<H3Command>,
    closed: Arc<AtomicBool>,
    open_streams: Arc<AtomicUsize>,
    pseudo_order: [PseudoOrder; 4],
}

impl H3Client {
    pub fn open_streams(&self) -> usize {
        self.open_streams.load(Ordering::Relaxed)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub async fn send_request(
        &self,
        method: &str,
        authority: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<Bytes>,
        body_stream: Option<H3RequestBodyStream>,
        mode: ResponseMode,
    ) -> Result<H3ResponseParts, H3SendError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H3SendError::NotSent("h3 connection closed".into()));
        }

        let uri_path = if path.is_empty() { "/" } else { path };
        let mut h3_headers: Vec<quiche::h3::Header> = Vec::with_capacity(4 + headers.len());
        for pseudo in self.pseudo_order {
            let (name, value): (&[u8], &[u8]) = match pseudo {
                PseudoOrder::Method => (b":method", method.as_bytes()),
                PseudoOrder::Scheme => (b":scheme", b"https"),
                PseudoOrder::Authority => (b":authority", authority.as_bytes()),
                PseudoOrder::Path => (b":path", uri_path.as_bytes()),
            };
            h3_headers.push(quiche::h3::Header::new(name, value));
        }
        for (k, v) in headers {
            h3_headers.push(quiche::h3::Header::new(k.as_bytes(), v.as_bytes()));
        }

        let (stream_body_tx, stream_body_rx) = if mode == ResponseMode::Buffered {
            (None, None)
        } else {
            let (tx, rx) = mpsc::channel(STREAM_RESP_CAPACITY + 1);
            (Some(tx), Some(rx))
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(H3Command::Request {
                headers: h3_headers,
                body,
                body_stream,
                stream_body_tx,
                mode,
                resp_tx,
                retried: false,
            })
            .await
            .map_err(|_| H3SendError::NotSent("h3 driver task has exited".into()))?;

        match resp_rx.await {
            Ok(Ok(head)) => Ok(H3ResponseParts {
                status: head.status,
                headers: head.headers,
                trailers: head.trailers,
                body: match stream_body_rx {
                    Some(rx) if mode.keeps_stream(head.status) => H3RespBody::Streaming(rx),
                    _ => H3RespBody::Buffered(head.body),
                },
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(H3SendError::Failed(
                "h3 driver dropped response sender".into(),
            )),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

struct H3Stream {
    resp_tx: Option<oneshot::Sender<Result<H3Response, H3SendError>>>,
    response: H3ResponseState,
    status: u16,
    headers: Vec<(String, String)>,
    trailers: Vec<(String, String)>,
    body: Vec<u8>,
    stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
    terminal: Option<mpsc::OwnedPermit<std::io::Result<Bytes>>>,
    mode: ResponseMode,
    head_sent: bool,
    stalled: Option<Bytes>,
    peer_finished: bool,
    body_bytes_seen: usize,
    declared_len: Option<u64>,
    expects_body: bool,
    out_chunks: VecDeque<Bytes>,
    out_offset: usize,
    body_eof: bool,
    fin_sent: bool,
    upload_credit: Option<Arc<Semaphore>>,
    pump: Option<AbortHandle>,
    retry: Option<(Vec<quiche::h3::Header>, Option<Bytes>)>,
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
