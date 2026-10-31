use std::sync::atomic::Ordering;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::{ErrorCode, H2Error};

use super::*;

const WRITE_STALL_TICKS: u32 = 10;
const QUEUED_WRITE_BATCH: u32 = 4;

#[derive(Default)]
pub(super) struct OutputState {
    goaway_sent: bool,
    progressed: bool,
    stall_ticks: u32,
    stalled: bool,
    deferred: u32,
    yielded: bool,
}

impl OutputState {
    pub(super) fn stalled(&self) -> bool {
        self.stalled
    }

    pub(super) fn wrote(&mut self, written: Result<(), H2Error>) -> Result<(), H2Error> {
        written?;
        self.progressed = true;
        self.deferred = 0;
        self.yielded = false;
        Ok(())
    }

    pub(super) fn coalesce(&mut self, has_output: bool) -> bool {
        let first = has_output && !self.yielded;
        self.yielded |= first;
        first
    }

    pub(super) fn defer(&mut self) {
        self.deferred = self.deferred.saturating_add(1);
    }

    pub(super) fn batch_full(&self) -> bool {
        self.deferred >= QUEUED_WRITE_BATCH
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn goaway_drained(&mut self) -> Result<bool, H2Error> {
        if !self.output.goaway_sent {
            self.output.goaway_sent = true;
            self.writer
                .write_frame(&GoAwayFrame {
                    last_stream_id: 0,
                    error_code: ErrorCode::NoError,
                    debug_data: Bytes::new(),
                })
                .await?;
        }
        Ok(!self.writer.has_output())
    }

    pub(super) fn check_write_stall(&mut self) -> Result<(), H2Error> {
        let blocked = self.writer.has_output() && !self.output.progressed;
        self.output.progressed = false;
        self.output.stall_ticks = if blocked {
            self.output.stall_ticks.saturating_add(1)
        } else {
            0
        };
        if self.output.stall_ticks < WRITE_STALL_TICKS {
            return Ok(());
        }
        if self.streams.is_empty() {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "peer stopped reading with no live streams".into(),
            });
        }
        if !self.output.stalled {
            self.output.stalled = true;
            self.closed.store(true, Ordering::Release);
        }
        Ok(())
    }
}
