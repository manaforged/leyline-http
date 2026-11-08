use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use super::handshake::wire_error;
use super::{Message, WsIo};
use crate::error::Result;

pub(super) struct WsSink {
    pub(super) inner: SplitSink<WebSocketStream<WsIo>, WireMessage>,
    pub(super) version: http::Version,
}

impl WsSink {
    pub(super) async fn send(&mut self, msg: Message) -> Result<()> {
        self.inner
            .send(msg.into_wire())
            .await
            .map_err(|e| wire_error("ws send", e))
    }
}

pub(super) struct WsStream {
    pub(super) inner: SplitStream<WebSocketStream<WsIo>>,
    pub(super) version: http::Version,
}

impl WsStream {
    pub(super) async fn recv(&mut self) -> Result<Option<Message>> {
        match self.inner.next().await {
            Some(Ok(msg)) => Ok(Some(Message::wire(msg))),
            Some(Err(e)) => Err(wire_error("ws recv", e)),
            None => Ok(None),
        }
    }
}
