//! [`H2Client`] — the cloneable public handle to a running HTTP/2
//! connection. Each clone shares one connection; requests fan out to the
//! driver task over an mpsc channel and run as independent streams.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::PollSender;

use crate::h2::connection::{H2Response, PseudoHeaders};
use crate::h2::error::{ErrorCode, H2Error};

use super::connect_stream::{H2ConnectStream, ShutdownState};
use super::driver::{
    DriverCommand, DriverRequestBody, PeerSettingsSnapshot, STREAM_REQ_BODY_CAPACITY,
    STREAM_RESP_BODY_CAPACITY, pump_request_body,
};
use super::types::{H2ResponseEx, RequestBody, ResponseBody};

/// Cloneable handle to a running HTTP/2 connection.
///
/// Every clone shares the same connection. Concurrent `send_request`
/// calls on one or many clones are multiplexed across independent
/// streams on the single underlying TCP connection with no
/// head-of-line blocking between streams.
#[derive(Clone)]
pub struct H2Client {
    pub(super) tx: mpsc::Sender<DriverCommand>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    /// Send a request over a multiplexed stream and await the response.
    ///
    /// Concurrent calls run in parallel on independent streams; one
    /// stream's flow-control stall does not block others.
    pub async fn send_request(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        self.send_request_with_trailers(pseudo, headers, body, Vec::new())
            .await
    }

    /// Send a request with optional trailers and await the response.
    ///
    /// Empty `trailers` is equivalent to [`Self::send_request`]. Non-empty
    /// trailers emit a terminating HEADERS frame with END_STREAM after
    /// the request body per RFC 9113 §8.1.
    pub async fn send_request_with_trailers(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
    ) -> Result<H2Response, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }

        let (response_tx, response_rx) = oneshot::channel();
        let cmd = DriverCommand::SendRequest {
            pseudo,
            headers,
            body,
            trailers,
            response_tx,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "driver task has exited".into(),
        })?;

        match response_rx.await {
            Ok(result) => result,
            Err(_) => Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver dropped response sender".into(),
            }),
        }
    }

    /// Extended send — supports streaming request bodies and optional
    /// streaming response delivery.
    ///
    /// When `body` is [`RequestBody::Streaming`], chunks are pumped to
    /// the driver via an mpsc channel; the driver honours flow-control
    /// as usual, parking the stream when the send window is empty.
    ///
    /// When `stream_response` is `true`, the oneshot resolves as soon as
    /// HEADERS arrive; body chunks are delivered via
    /// [`ResponseBody::Streaming`]. The caller is responsible for
    /// draining the receiver — the driver applies back-pressure through
    /// the bounded channel.
    pub async fn send_request_ex(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: RequestBody,
        stream_response: bool,
    ) -> Result<H2ResponseEx, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }

        // Convert the caller-supplied stream into an mpsc receiver that
        // the driver can pull from. A small producer task owns the
        // `Stream` object — the driver never awaits on user code.
        let body_in = match body {
            RequestBody::None => DriverRequestBody::None,
            RequestBody::Buffered(b) => DriverRequestBody::Buffered(b),
            RequestBody::Streaming {
                stream,
                length_hint,
            } => {
                let (body_tx, body_rx) = mpsc::channel(STREAM_REQ_BODY_CAPACITY);
                tokio::spawn(pump_request_body(stream, body_tx));
                DriverRequestBody::Streaming {
                    rx: body_rx,
                    length_hint,
                }
            }
        };

        // Streaming-response channel is created here so the caller keeps
        // the receiver while the driver only sees the sender. When the
        // driver resolves the oneshot, we stitch the receiver into the
        // returned `H2ResponseEx`.
        let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (body_body_tx, body_body_rx) =
            mpsc::channel::<io::Result<Bytes>>(STREAM_RESP_BODY_CAPACITY);

        let cmd = DriverCommand::SendRequestEx {
            pseudo,
            headers,
            body: body_in,
            stream_response,
            response_tx,
            stream_body_tx: body_body_tx,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "driver task has exited".into(),
        })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if stream_response {
                    // Replace the body payload with the receiver the
                    // caller holds. The driver's ResponseBody on the
                    // oneshot is a placeholder.
                    resp.body = ResponseBody::Streaming(body_body_rx);
                }
                Ok(resp)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver dropped response sender".into(),
            }),
        }
    }

    /// Return `true` if the driver task has shut down (GOAWAY, IO error,
    /// or last handle dropped).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Return `true` if the peer has advertised
    /// `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1` (RFC 8441 §3). Callers
    /// consult this before attempting an extended-CONNECT open;
    /// `false` (the default) means the server speaks only classic
    /// CONNECT and WebSocket clients should fall back to a fresh
    /// HTTP/1.1 TLS connection.
    pub fn peer_enables_connect_protocol(&self) -> bool {
        self.peer_settings.enable_connect_protocol()
    }

    /// Open an HTTP/2 extended CONNECT (RFC 8441) bidirectional stream.
    ///
    /// On success, returns an [`H2ConnectStream`] that implements
    /// `AsyncRead + AsyncWrite` over the pooled connection. Inbound
    /// DATA frames are surfaced as bytes to the reader; writes are
    /// chunked into DATA frames that honour flow control. Dropping
    /// the returned stream emits an END_STREAM DATA frame.
    ///
    /// Errors immediately if the peer has not advertised
    /// [`SETTINGS_ENABLE_CONNECT_PROTOCOL`](
    /// crate::h2::config::SETTINGS_ENABLE_CONNECT_PROTOCOL). Callers should
    /// fall back to the HTTP/1.1 WebSocket path in that case.
    pub async fn open_extended_connect(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
    ) -> Result<H2ConnectStream, H2Error> {
        if !self.peer_enables_connect_protocol() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "peer did not advertise SETTINGS_ENABLE_CONNECT_PROTOCOL=1; \
                         fall back to the HTTP/1.1 path"
                    .into(),
            });
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        if !pseudo.method.eq_ignore_ascii_case("CONNECT") || pseudo.protocol.is_none() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "extended CONNECT requires :method=CONNECT and :protocol".into(),
            });
        }

        let (write_tx, write_rx) = mpsc::channel::<io::Result<Bytes>>(STREAM_REQ_BODY_CAPACITY);
        let (body_tx, body_rx) = mpsc::channel::<io::Result<Bytes>>(STREAM_RESP_BODY_CAPACITY);
        let (headers_tx, headers_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();

        let cmd = DriverCommand::OpenConnect {
            pseudo,
            headers,
            write_rx,
            headers_tx,
            body_tx,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "driver task has exited".into(),
        })?;

        let resp = match headers_rx.await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(H2Error::Connection {
                    code: ErrorCode::NoError,
                    reason: "driver dropped response sender".into(),
                });
            }
        };

        #[allow(clippy::needless_update)]
        Ok(H2ConnectStream {
            shutdown_state: ShutdownState::Open,
            status: resp.status,
            response_headers: resp
                .headers
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.as_str().to_owned()))
                .collect(),
            write_tx: Some(PollSender::new(write_tx)),
            read_rx: body_rx,
            read_leftover: Bytes::new(),
            read_eof: false,
        })
    }
}
