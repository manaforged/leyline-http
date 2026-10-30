use std::time::Duration;

use bytes::Bytes;

use super::{CURRENT, Ctx, Event, ms};

pub(crate) struct BodyWatch {
    ctx: Ctx,
    bytes: u64,
    ended: bool,
}

impl BodyWatch {
    pub(crate) fn begin() -> Option<Self> {
        CURRENT
            .try_with(|ctx| Self {
                ctx: ctx.clone(),
                bytes: 0,
                ended: false,
            })
            .ok()
    }

    pub(crate) fn observe(&mut self, item: Option<&std::io::Result<Bytes>>) {
        match item {
            Some(Ok(chunk)) => self.bytes += chunk.len() as u64,
            Some(Err(err)) => self.end(Err(Some(err))),
            None => self.end(Ok(())),
        }
    }

    fn end(&mut self, outcome: Result<(), Option<&std::io::Error>>) {
        if self.ended {
            return;
        }
        self.ended = true;
        let (id, start) = self.ctx.root;
        self.ctx.hook.event(&Event::Body {
            id,
            bytes: self.bytes,
            elapsed: start.elapsed(),
            outcome,
        });
    }
}

impl Drop for BodyWatch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.end(Err(None));
        }
    }
}

pub(super) fn render(
    id: u64,
    bytes: u64,
    elapsed: Duration,
    outcome: Result<(), Option<&std::io::Error>>,
) {
    let elapsed_ms = ms(elapsed);
    match outcome {
        Ok(()) => {
            tracing::info!(target: "leyline::trace", id, bytes, elapsed_ms, outcome = "complete", "body")
        }
        Err(Some(err)) => {
            tracing::info!(target: "leyline::trace", id, bytes, elapsed_ms, outcome = "error", kind = ?err.kind(), error = %err, "body")
        }
        Err(None) => {
            tracing::info!(target: "leyline::trace", id, bytes, elapsed_ms, outcome = "dropped", "body")
        }
    }
}
