#[cfg(feature = "websocket")]
use std::io;
use std::sync::Arc;
use std::task::{Context, Poll};

#[cfg(feature = "websocket")]
use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use crate::core::Body;
use crate::h2::connection::{HeaderPair, PseudoHeaders};

use super::ResponseSink;

pub(crate) const MAX_FLOW_WINDOW: i64 = 0x7FFF_FFFF;

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
    pub headers: Vec<HeaderPair>,
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
    },
    Ping {
        ack_tx: oneshot::Sender<()>,
        deadline: std::time::Instant,
    },
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

#[derive(Clone)]
pub(crate) struct CommandTx(mpsc::Sender<CommandSlot>);

pub(super) struct CommandRx(mpsc::Receiver<CommandSlot>);

pub(super) fn command_channel(capacity: usize) -> (CommandTx, CommandRx) {
    let (tx, rx) = mpsc::channel(capacity);
    (CommandTx(tx), CommandRx(rx))
}

impl CommandTx {
    pub(crate) async fn send(
        &self,
        cmd: DriverCommand,
    ) -> Result<(), mpsc::error::SendError<DriverCommand>> {
        self.0
            .send(Box::new(cmd))
            .await
            .map_err(|err| mpsc::error::SendError(*err.0))
    }
}

impl CommandRx {
    pub(super) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<DriverCommand>> {
        self.0.poll_recv(cx).map(|cmd| cmd.map(|boxed| *boxed))
    }

    pub(super) fn try_recv(&mut self) -> Result<DriverCommand, mpsc::error::TryRecvError> {
        self.0.try_recv().map(|boxed| *boxed)
    }

    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
}

impl DriverCommand {
    pub(crate) fn is_cancelled(&self) -> bool {
        match self {
            DriverCommand::SendRequest { sink, .. } => sink.is_cancelled(),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => sink.is_cancelled(),
            DriverCommand::Ping { .. } => false,
        }
    }

    pub(crate) fn into_sink(self) -> Option<ResponseSink> {
        match self {
            DriverCommand::SendRequest { sink, .. } => Some(sink),
            #[cfg(feature = "websocket")]
            DriverCommand::OpenConnect { sink, .. } => Some(sink),
            DriverCommand::Ping { .. } => None,
        }
    }
}

#[cfg(test)]
mod command_channel_tests;
#[cfg(kani)]
mod flow_control_proofs;
#[cfg(test)]
mod flow_control_tests;
