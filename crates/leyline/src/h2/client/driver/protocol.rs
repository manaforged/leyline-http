#[cfg(feature = "websocket")]
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

#[cfg(feature = "websocket")]
use bytes::Bytes;
use tokio::sync::{Notify, mpsc};

use crate::h2::H2Error;
#[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
use crate::h2::connection::PseudoHeaders;
use crate::h2::connection::{PseudoList, build_pseudo_fields};
use crate::header_str::HeaderStr;
use crate::profile::PseudoOrder;
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

enum RequestTarget {
    Url {
        method: HeaderStr,
        url: Arc<url::Url>,
    },
    #[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
    Pseudo(Box<PseudoHeaders>),
}

pub struct Head {
    target: RequestTarget,
    pub(crate) headers: crate::wire::RequestHeaders,
    pub(crate) body_limit: crate::error::BodyLimit,
}

impl Head {
    #[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
    /// Builds a request head from its pseudo-headers and header fields.
    pub fn new(pseudo: PseudoHeaders, headers: Vec<crate::wire::HeaderPair>) -> Self {
        Self {
            target: RequestTarget::Pseudo(Box::new(pseudo)),
            headers: headers.into(),
            body_limit: crate::error::BodyLimit::session(crate::wire::DEFAULT_MAX_BODY_SIZE),
        }
    }

    pub(crate) fn from_url(
        method: HeaderStr,
        url: Arc<url::Url>,
        headers: crate::wire::RequestHeaders,
        body_limit: crate::error::BodyLimit,
    ) -> Self {
        Self {
            target: RequestTarget::Url { method, url },
            headers,
            body_limit,
        }
    }

    pub(crate) fn method_is(&self, method: &http::Method) -> bool {
        match &self.target {
            RequestTarget::Url { method: m, .. } => m.eq_ignore_ascii_case(method.as_str()),
            #[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
            RequestTarget::Pseudo(pseudo) => pseudo.method_is(method),
        }
    }

    pub(crate) fn build_pseudo_list<'a>(
        &'a self,
        pseudo_order: &[PseudoOrder; 4],
    ) -> Result<PseudoList<'a>, H2Error> {
        match &self.target {
            RequestTarget::Url { method, url } => build_pseudo_fields(
                method.as_str(),
                "https",
                &url[url::Position::BeforeHost..url::Position::AfterPort],
                crate::util::request_target(url),
                None,
                pseudo_order,
            ),
            #[cfg(any(test, feature = "bench-internals", feature = "websocket"))]
            RequestTarget::Pseudo(pseudo) => pseudo.build_pseudo_list(pseudo_order),
        }
    }
}

#[cfg(feature = "websocket")]
pub(crate) struct OpenConnect {
    pub(crate) head: Box<Head>,
    pub(crate) write_rx: mpsc::Receiver<io::Result<Bytes>>,
    pub(crate) sink: ResponseSink,
    pub(crate) slot: Option<super::super::handle::Slot>,
}

pub(crate) enum DriverCommand {
    SendRequest {
        head: Arc<Head>,
        body: Body,
        sink: ResponseSink,
        slot: Option<super::super::handle::Slot>,
    },
    #[cfg(feature = "websocket")]
    OpenConnect(Box<OpenConnect>),
    Ping(super::Ping),
}

const _: () = assert!(
    size_of::<DriverCommand>() <= 80,
    "DriverCommand is stored inline in the unbounded channel without per-request boxing"
);

pub(super) type CommandSlot = DriverCommand;

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
        let mut waited = false;
        loop {
            if self.queue.queued.fetch_add(1, Ordering::AcqRel) < self.capacity {
                let sent = self.tx.send(cmd).map_err(|err| {
                    self.queue.queued.fetch_sub(1, Ordering::AcqRel);
                    mpsc::error::SendError(err.0)
                });
                if waited && self.queue.queued.load(Ordering::Acquire) < self.capacity {
                    self.queue.space.notify_one();
                }
                return sent;
            }
            self.queue.queued.fetch_sub(1, Ordering::AcqRel);
            let space = self.queue.space.notified();
            tokio::pin!(space);
            space.as_mut().enable();
            if self.tx.is_closed() {
                return Err(mpsc::error::SendError(cmd));
            }
            if self.queue.queued.load(Ordering::Acquire) >= self.capacity {
                space.await;
                waited = true;
            }
        }
    }
}

impl Drop for CommandRx {
    fn drop(&mut self) {
        self.rx.close();
        self.queue.space.notify_waiters();
    }
}

impl CommandRx {
    pub(super) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<DriverCommand>> {
        self.rx.poll_recv(cx).map(|cmd| cmd.map(|c| self.taken(c)))
    }

    pub(super) fn try_recv(&mut self) -> Result<DriverCommand, mpsc::error::TryRecvError> {
        self.rx.try_recv().map(|c| self.taken(c))
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
            DriverCommand::OpenConnect(connect) => connect.sink.is_cancelled(),
            DriverCommand::Ping(_) => false,
        }
    }

    pub(crate) fn into_sink(self) -> Option<ResponseSink> {
        match self {
            DriverCommand::SendRequest { sink, .. } => Some(sink),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect(connect) => Some(connect.sink),
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
