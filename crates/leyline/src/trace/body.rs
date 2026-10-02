use std::time::Duration;

use bytes::Bytes;

use super::{CURRENT, Ctx, ms};

#[derive(Debug)]
#[non_exhaustive]
pub enum BodyOutcome<'a> {
    Complete,
    Failed(&'a std::io::Error),
    Dropped,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct BodyEnd<'a> {
    pub id: u64,
    pub bytes: u64,
    pub elapsed: Duration,
    pub outcome: BodyOutcome<'a>,
}

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
            Some(Err(err)) => self.end(BodyOutcome::Failed(err)),
            None => self.end(BodyOutcome::Complete),
        }
    }

    fn end(&mut self, outcome: BodyOutcome<'_>) {
        if self.ended {
            return;
        }
        self.ended = true;
        let (id, start) = self.ctx.root;
        self.ctx.hook.body(&BodyEnd {
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
            self.end(BodyOutcome::Dropped);
        }
    }
}

pub(super) fn render(ev: &BodyEnd<'_>) {
    let elapsed_ms = ms(ev.elapsed);
    match ev.outcome {
        BodyOutcome::Complete => {
            tracing::info!(target: "leyline::trace", id = ev.id, bytes = ev.bytes, elapsed_ms, outcome = "complete", "body")
        }
        BodyOutcome::Failed(err) => {
            tracing::info!(target: "leyline::trace", id = ev.id, bytes = ev.bytes, elapsed_ms, outcome = "error", kind = ?err.kind(), error = %err, "body")
        }
        BodyOutcome::Dropped => {
            tracing::info!(target: "leyline::trace", id = ev.id, bytes = ev.bytes, elapsed_ms, outcome = "dropped", "body")
        }
    }
}
