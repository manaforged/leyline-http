use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use super::Message;
use super::handshake::wire_error;
use crate::core::error::Result;
use crate::h2::client::H2ConnectStream;
use crate::tls::TlsIo;

pub(super) enum WsSinkInner {
    H1(SplitSink<WebSocketStream<TlsIo>, WireMessage>),
    H2(SplitSink<WebSocketStream<H2ConnectStream>, WireMessage>),
}

pub(super) struct WsSink {
    pub(super) inner: WsSinkInner,
}

impl WsSink {
    pub(super) async fn send(&mut self, msg: Message) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsSinkInner::H1(s) => s.send(msg).await,
            WsSinkInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| wire_error("ws send", e))
    }
}

pub(super) enum WsStreamInner {
    H1(SplitStream<WebSocketStream<TlsIo>>),
    H2(SplitStream<WebSocketStream<H2ConnectStream>>),
}

pub(super) struct WsStream {
    pub(super) inner: WsStreamInner,
}

impl WsStream {
    pub(super) async fn recv(&mut self) -> Result<Option<Message>> {
        let next = match &mut self.inner {
            WsStreamInner::H1(s) => s.next().await,
            WsStreamInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(Message::wire(msg))),
            Some(Err(e)) => Err(wire_error("ws recv", e)),
            None => Ok(None),
        }
    }
}
