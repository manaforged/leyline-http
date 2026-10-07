use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::h2::error::{ErrorCode, H2Error};

#[cfg(feature = "websocket")]
use super::driver::PeerSettingsSnapshot;
use super::driver::{DriverCommand, DriverRequestBody, Head, ResponseSink};
use super::types::{H2ResponseEx, RequestBody, ResponseBody};
use crate::core::ResponseMode;

#[derive(Clone)]
pub struct H2Client {
    pub(super) tx: mpsc::Sender<DriverCommand>,
    pub(super) ping_tx: mpsc::Sender<oneshot::Sender<()>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) open_streams: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(feature = "websocket")]
    pub(super) peer_settings: Arc<PeerSettingsSnapshot>,
}

impl H2Client {
    pub fn open_streams(&self) -> usize {
        self.open_streams.load(Ordering::Relaxed)
    }

    pub async fn send_shared(
        &self,
        head: Arc<Head>,
        body: RequestBody,
        mode: impl Into<ResponseMode>,
    ) -> Result<H2ResponseEx, H2Error> {
        let mode = mode.into();
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Stream {
                stream_id: 0,
                code: ErrorCode::RefusedStream,
            });
        }

        let body_in = match body {
            RequestBody::None => DriverRequestBody::None,
            RequestBody::Buffered(b) => DriverRequestBody::Buffered(b),
            RequestBody::Streaming { stream, .. } => DriverRequestBody::Streaming(stream),
        };

        let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponseEx, H2Error>>();
        let (sink, stream_body_rx) = match mode {
            ResponseMode::Buffered => (ResponseSink::Buffered(response_tx), None),
            ResponseMode::Streamed => {
                let (sink, receiver) = ResponseSink::streaming(response_tx);
                (sink, Some(receiver))
            }
            ResponseMode::ErrorPrefix(_) => {
                let (sink, receiver) = ResponseSink::adaptive(response_tx, mode);
                (sink, Some(receiver))
            }
        };

        let cmd = DriverCommand::SendRequest {
            head,
            body: body_in,
            sink,
        };
        self.tx.send(cmd).await.map_err(|_| H2Error::Stream {
            stream_id: 0,
            code: ErrorCode::RefusedStream,
        })?;

        match response_rx.await {
            Ok(Ok(mut resp)) => {
                if let Some(rx) = stream_body_rx
                    && mode.keeps_stream(resp.status)
                {
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

    pub async fn ping(&self) -> Result<(), H2Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "connection closed".into(),
            });
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        self.ping_tx
            .send(ack_tx)
            .await
            .map_err(|_| H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "driver task has exited".into(),
            })?;
        ack_rx.await.map_err(|_| H2Error::Connection {
            code: ErrorCode::NoError,
            reason: "connection closed before the ping was acknowledged".into(),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}
