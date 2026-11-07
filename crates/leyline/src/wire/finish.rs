use std::io;

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use tokio::sync::mpsc;

use super::{BodyChannel, Fields, ResponseMode, ResponseTiming, TransportBody, TransportResponse};

/// One streamed response body item: a chunk or the error that ends the body.
pub(crate) type Chunk = io::Result<Bytes>;

/// How many chunks a streamed response body holds before the transport waits.
pub(crate) const STREAM_CHANNEL_DEPTH: usize = 32;

/// Returns whether a response with `status` to a request carries a body; a HEAD request never gets one.
#[inline]
pub(crate) const fn has_body(head_request: bool, status: u16) -> bool {
    !head_request && !matches!(status, 204 | 304)
}

/// Returns whether a body of `received` bytes disagrees with the declared content-length.
#[inline]
pub(crate) fn length_mismatch(has_body: bool, declared: Option<u64>, received: u64) -> bool {
    has_body && declared.is_some_and(|len| len != received)
}

/// Opens a streamed body channel and holds one slot back for the item that ends the body.
pub(crate) fn body_channel() -> (
    mpsc::Sender<Chunk>,
    mpsc::OwnedPermit<Chunk>,
    mpsc::Receiver<Chunk>,
) {
    let (tx, rx) = mpsc::channel(STREAM_CHANNEL_DEPTH + 1);
    let terminal = tx
        .clone()
        .try_reserve_owned()
        .expect("a new body channel has a free slot");
    (tx, terminal, rx)
}

impl TransportResponse {
    pub(crate) fn new(
        version: http::Version,
        status: StatusCode,
        headers: Fields,
        trailers: HeaderMap,
        body: TransportBody,
    ) -> Self {
        Self {
            status,
            headers,
            trailers: super::trailers(trailers),
            body,
            version,
            tls: None,
            timing: ResponseTiming::default(),
        }
    }

    pub(crate) fn streamed(
        mut self,
        mode: ResponseMode,
        rx: Option<mpsc::Receiver<Chunk>>,
    ) -> Self {
        if let Some(rx) = rx
            && mode.keeps_stream(self.status.as_u16())
        {
            self.body = TransportBody::Streaming(BodyChannel::new(rx));
        }
        self
    }
}
