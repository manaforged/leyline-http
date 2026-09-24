use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use leyline_quiche as quiche;
use quiche::h3::NameValue;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::pool::TlsInfo;
use crate::profile::BrowserProfile;
use crate::quic::config::H3Config;
use crate::quic::connection::{
    EstablishedH3, H3Response, check_body_budget, close_reason, connect_and_handshake, flush_egress,
};
use crate::tls::{Resolver, TlsTrustConfig};

const COMMAND_CHANNEL_CAPACITY: usize = 1024;

const STREAM_RESP_CAPACITY: usize = 32;

const STREAM_REQ_CAPACITY: usize = 64;

const UPLOAD_WINDOW: usize = 256 * 1024;

const UPLOAD_CHUNK: usize = 16 * 1024;

const STREAM_PUMP_INTERVAL: Duration = Duration::from_millis(2);

const CANCEL_SWEEP_INTERVAL: Duration = Duration::from_millis(100);

const LENGTH_MISMATCH: &str = "h3: response body length does not match content-length";

pub type H3RequestBodyStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

enum H3BodyChunk {
    Chunk {
        stream_id: u64,
        data: Bytes,
    },
    Eof {
        stream_id: u64,
        error: Option<std::io::Error>,
    },
}

enum H3Command {
    Request {
        headers: Vec<quiche::h3::Header>,
        body: Option<Bytes>,
        body_stream: Option<H3RequestBodyStream>,
        stream_body_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
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
    Failed(String),
}

impl H3SendError {
    pub(crate) fn message(&self) -> &str {
        match self {
            H3SendError::NotSent(m) | H3SendError::Failed(m) => m,
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
}

impl H3Client {
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
        stream_response: bool,
    ) -> Result<H3ResponseParts, H3SendError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H3SendError::NotSent("h3 connection closed".into()));
        }

        let uri_path = if path.is_empty() { "/" } else { path };
        let mut h3_headers: Vec<quiche::h3::Header> = Vec::with_capacity(4 + headers.len());
        h3_headers.push(quiche::h3::Header::new(b":method", method.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":scheme", b"https"));
        h3_headers.push(quiche::h3::Header::new(b":authority", authority.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":path", uri_path.as_bytes()));
        for (k, v) in headers {
            h3_headers.push(quiche::h3::Header::new(k.as_bytes(), v.as_bytes()));
        }

        let (stream_body_tx, stream_body_rx) = if stream_response {
            let (tx, rx) = mpsc::channel(STREAM_RESP_CAPACITY);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(H3Command::Request {
                headers: h3_headers,
                body,
                body_stream,
                stream_body_tx,
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
                    Some(rx) => H3RespBody::Streaming(rx),
                    None => H3RespBody::Buffered(head.body),
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

pub struct H3DriverTask(
    #[expect(
        dead_code,
        reason = "field held so the JoinHandle drops (and thus never aborts) with the struct; never read"
    )]
    tokio::task::JoinHandle<()>,
);

struct H3Stream {
    resp_tx: Option<oneshot::Sender<Result<H3Response, H3SendError>>>,
    response: H3ResponseState,
    status: u16,
    headers: Vec<(String, String)>,
    trailers: Vec<(String, String)>,
    body: Vec<u8>,
    stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
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

impl Drop for H3Stream {
    fn drop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
    }
}

impl H3Stream {
    fn new(
        resp_tx: oneshot::Sender<Result<H3Response, H3SendError>>,
        body: Option<Bytes>,
        stream_tx: Option<mpsc::Sender<std::io::Result<Bytes>>>,
        streaming: bool,
    ) -> Self {
        let mut out_chunks = VecDeque::new();
        let (body_eof, fin_sent) = if streaming {
            (false, false)
        } else {
            match body {
                Some(b) if !b.is_empty() => {
                    out_chunks.push_back(b);
                    (true, false)
                }
                _ => (true, true),
            }
        };
        Self {
            resp_tx: Some(resp_tx),
            response: H3ResponseState::Initial,
            status: 0,
            headers: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
            stream_tx,
            head_sent: false,
            stalled: None,
            peer_finished: false,
            body_bytes_seen: 0,
            declared_len: None,
            expects_body: true,
            out_chunks,
            out_offset: 0,
            body_eof,
            fin_sent,
            upload_credit: None,
            pump: None,
            retry: None,
        }
    }

    fn cancel_upload(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        self.out_chunks.clear();
        self.out_offset = 0;
        self.fin_sent = true;
    }

    fn is_streaming(&self) -> bool {
        self.stream_tx.is_some()
    }

    fn body_write_pending(&self) -> bool {
        !self.out_chunks.is_empty() || (self.body_eof && !self.fin_sent)
    }

    fn send_side_open(&self) -> bool {
        !self.fin_sent
    }

    fn headers(&mut self, list: &[(String, String)]) -> Result<(), &'static str> {
        match self.response.headers(list)? {
            H3HeaderBlock::Informational => {}
            H3HeaderBlock::Final { status, headers } => {
                self.status = status;
                self.declared_len = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse().ok());
                self.headers = headers;
            }
            H3HeaderBlock::Trailers(headers) => self.trailers = headers,
        }
        Ok(())
    }

    fn data(&self) -> Result<(), &'static str> {
        self.response.data()
    }

    fn finish(&self) -> Result<(), &'static str> {
        self.response.finish()?;
        if !self.is_streaming() && self.length_mismatch() {
            return Err(LENGTH_MISMATCH);
        }
        Ok(())
    }

    fn length_mismatch(&self) -> bool {
        self.expects_body
            && !matches!(self.status, 204 | 304)
            && self
                .declared_len
                .is_some_and(|len| len != self.body_bytes_seen as u64)
    }

    fn deliver(&mut self, result: Result<H3Response, String>) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(result.map_err(H3SendError::Failed)));
        }
    }

    fn deliver_unsent(&mut self, message: String) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Err(H3SendError::NotSent(message))));
        }
    }

    fn deliver_head(&mut self) {
        if let Some(tx) = self.resp_tx.take() {
            drop(tx.send(Ok(H3Response {
                status: self.status,
                headers: std::mem::take(&mut self.headers),
                body: Vec::new(),
                trailers: Vec::new(),
            })));
        }
        self.head_sent = true;
    }

    fn deliver_error(&mut self, message: String) {
        if self.head_sent {
            if let Some(tx) = &self.stream_tx {
                deliver_stream_error(tx, std::io::Error::other(message));
            }
        } else {
            self.deliver(Err(message));
        }
    }
}

pub(crate) async fn open_fresh_h3(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    resolver: &dyn Resolver,
    host: &str,
    port: u16,
) -> Result<(H3Client, H3DriverTask, TlsInfo), String> {
    let established = connect_and_handshake(h3_cfg, profile, trust, resolver, host, port).await?;
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
    let task = tokio::spawn(driver.run());

    Ok((H3Client { tx, closed }, H3DriverTask(task), tls))
}

struct H3Driver {
    established: EstablishedH3,
    command_rx: mpsc::Receiver<H3Command>,
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    streams: HashMap<u64, H3Stream>,
}

mod driver;
use driver::*;
mod response;
use response::*;

#[cfg(test)]
mod tests;
