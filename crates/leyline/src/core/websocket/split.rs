use futures_util::stream::{SplitSink, SplitStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use super::{Message, WsIo, recv_on, send_on};
use crate::error::Result;

pub(super) struct WsSink {
    pub(super) inner: SplitSink<WebSocketStream<WsIo>, WireMessage>,
    pub(super) version: http::Version,
}

impl WsSink {
    pub(super) async fn send(&mut self, msg: Message) -> Result<()> {
        send_on(&mut self.inner, msg).await
    }
}

pub(super) struct WsStream {
    pub(super) inner: SplitStream<WebSocketStream<WsIo>>,
    pub(super) version: http::Version,
}

impl WsStream {
    pub(super) async fn recv(&mut self) -> Result<Option<Message>> {
        recv_on(&mut self.inner).await
    }
}
