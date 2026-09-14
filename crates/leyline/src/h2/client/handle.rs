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
    DriverCommand, DriverRequestBody, Head, PeerSettingsSnapshot, ResponseSink,
    STREAM_REQ_BODY_CAPACITY, pump_request_body,
};
use super::types::{H2ResponseEx, RequestBody, ResponseBody};

#[derive(Clone)]
pub struct H2Client {
    pub(super) tx: mpsc::Sender<DriverCommand>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    pub async fn send_request(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        self.send_request_with_trailers(pseudo, headers, body, Vec::new())
            .await
    }

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

    pub async fn send_request_ex(
        &self,
        pseudo: PseudoHeaders,
        headers: Vec<crate::h2::connection::HeaderPair>,
        body: RequestBody,
        stream_response: bool,
    ) -> Result<H2ResponseEx, H2Error> {
        self.send_shared(Arc::new(Head { pseudo, headers }), body, stream_response)
            .await
    }

    pub(crate) async fn send_shared(
        &self,
        head: Arc<Head>,
        body: RequestBody,
        stream_response: bool,
    ) -> Result<H2ResponseEx, H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }

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

        let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (sink, stream_body_rx) = if stream_response {
            let (sink, receiver) = ResponseSink::streaming(response_tx);
            (sink, Some(receiver))
        } else {
            (ResponseSink::BufferedEx(response_tx), None)
        };

        let cmd = DriverCommand::SendRequestEx {
            head,
            body: body_in,
            sink,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "driver task has exited".into(),
        })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if let Some(rx) = stream_body_rx {
                    resp.body = ResponseBody::Streaming(rx);
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

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn peer_enables_connect_protocol(&self) -> bool {
        self.peer_settings.enable_connect_protocol()
    }

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
        let (headers_tx, headers_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (sink, body_rx) = ResponseSink::streaming(headers_tx);

        let cmd = DriverCommand::OpenConnect {
            pseudo,
            headers,
            write_rx,
            sink,
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
