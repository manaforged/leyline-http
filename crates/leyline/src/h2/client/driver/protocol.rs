#[cfg(feature = "websocket")]
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

#[cfg(feature = "websocket")]
use bytes::Bytes;
use tokio::sync::{Notify, mpsc};

use crate::h2::connection::PseudoHeaders;
use crate::wire::Body;

use super::ResponseSink;

pub(crate) const MAX_FLOW_WINDOW: i64 = crate::h2::frame::U31_MAX as i64;

pub(crate) fn checked_window_add(current: i64, delta: i64) -> Result<i64, i64> {
    let new_win = current.saturating_add(delta);
    if new_win > MAX_FLOW_WINDOW {
        Err(new_win)
    } else {
        Ok(new_win)
    }
}

pub struct Head {
    pub pseudo: PseudoHeaders,
    pub(crate) headers: crate::wire::RequestHeaders,
    pub(crate) body_limit: crate::error::BodyLimit,
}

impl Head {
    #[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
    /// Builds a request head from its pseudo-headers and header fields.
    pub fn new(pseudo: PseudoHeaders, headers: Vec<crate::wire::HeaderPair>) -> Self {
        Self {
            pseudo,
            headers: headers.into(),
            body_limit: crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        }
    }
}

pub(crate) enum DriverCommand {
    SendRequest {
        head: Arc<Head>,
        body: Body,
        sink: ResponseSink,
        slot: Option<super::super::handle::Slot>,
    },
    #[cfg(feature = "websocket")]
    OpenConnect {
        head: Box<Head>,
        write_rx: mpsc::Receiver<io::Result<Bytes>>,
        sink: ResponseSink,
        slot: Option<super::super::handle::Slot>,
    },
    Ping(super::Ping),
}

const _: () = assert!(
    size_of::<DriverCommand>() <= 112,
    "DriverCommand is copied into the driver queue after the channel unboxes it"
);

pub(super) type CommandSlot = Box<DriverCommand>;

const _: () = assert!(
    size_of::<CommandSlot>() == size_of::<usize>(),
    "the command channel stores a box, not the enum"
);

#[derive(Default)]
struct Queue {
    queued: AtomicUsize,
    space: Notify,
}

#[derive(Clone)]
pub(crate) struct CommandTx {
    tx: mpsc::UnboundedSender<CommandSlot>,
    queue: Arc<Queue>,
    capacity: usize,
}

pub(super) struct CommandRx {
    rx: mpsc::UnboundedReceiver<CommandSlot>,
    queue: Arc<Queue>,
    capacity: usize,
}

pub(super) fn command_channel(capacity: usize) -> (CommandTx, CommandRx) {
    let (tx, rx) = mpsc::unbounded_channel();
    let queue = Arc::new(Queue::default());
    (
        CommandTx {
            tx,
            queue: Arc::clone(&queue),
            capacity,
        },
        CommandRx {
            rx,
            queue,
            capacity,
        },
    )
}

impl CommandTx {
    pub(crate) async fn send(
        &self,
        cmd: DriverCommand,
    ) -> Result<(), mpsc::error::SendError<DriverCommand>> {
        loop {
            if self.queue.queued.fetch_add(1, Ordering::AcqRel) < self.capacity {
                return self.tx.send(Box::new(cmd)).map_err(|err| {
                    self.queue.queued.fetch_sub(1, Ordering::AcqRel);
                    mpsc::error::SendError(*err.0)
                });
            }
            self.queue.queued.fetch_sub(1, Ordering::AcqRel);
            if self.tx.is_closed() {
                return Err(mpsc::error::SendError(cmd));
            }
            let space = self.queue.space.notified();
            tokio::pin!(space);
            space.as_mut().enable();
            if self.queue.queued.load(Ordering::Acquire) < self.capacity {
                continue;
            }
            space.await;
        }
    }
}

impl CommandRx {
    pub(super) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<DriverCommand>> {
        self.rx
            .poll_recv(cx)
            .map(|cmd| cmd.map(|boxed| self.taken(*boxed)))
    }

    pub(super) fn try_recv(&mut self) -> Result<DriverCommand, mpsc::error::TryRecvError> {
        self.rx.try_recv().map(|boxed| self.taken(*boxed))
    }

    fn taken(&self, cmd: DriverCommand) -> DriverCommand {
        if self.queue.queued.fetch_sub(1, Ordering::AcqRel) >= self.capacity {
            self.queue.space.notify_one();
        }
        cmd
    }

    pub(super) fn len(&self) -> usize {
        self.rx.len()
    }
}

impl DriverCommand {
    pub(crate) fn is_cancelled(&self) -> bool {
        match self {
            DriverCommand::SendRequest { sink, .. } => sink.is_cancelled(),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => sink.is_cancelled(),
            DriverCommand::Ping(_) => false,
        }
    }

    pub(crate) fn into_sink(self) -> Option<ResponseSink> {
        match self {
            DriverCommand::SendRequest { sink, .. } => Some(sink),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => Some(sink),
            DriverCommand::Ping(_) => None,
        }
    }
}

#[cfg(test)]
mod command_channel_tests;
#[cfg(kani)]
mod flow_control_proofs;
#[cfg(test)]
mod flow_control_tests;
