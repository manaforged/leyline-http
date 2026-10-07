use std::sync::atomic::Ordering;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::error::{ErrorCode, H2Error};

use super::*;

const WRITE_STALL_TICKS: u32 = 10;

#[derive(Default)]
pub(super) struct OutputState {
    goaway_sent: bool,
    progressed: bool,
    stall_ticks: u32,
    stalled: bool,
}

impl OutputState {
    pub(super) fn stalled(&self) -> bool {
        self.stalled
    }

    pub(super) fn wrote(&mut self, written: Result<(), H2Error>) -> Result<(), H2Error> {
        written?;
        self.progressed = true;
        Ok(())
    }
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn goaway_drained(&mut self) -> Result<bool, H2Error> {
        if !self.output.goaway_sent {
            self.output.goaway_sent = true;
            self.writer.write_goaway(0, ErrorCode::NoError).await?;
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
