use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use super::WsMessage;
use crate::core::error::{Error, Kind, Result};
use crate::h2::client::H2ConnectStream;
use crate::tls::TlsIo;

pub(super) enum WsSinkInner {
    H1(SplitSink<WebSocketStream<TlsIo>, Message>),
    H2(SplitSink<WebSocketStream<H2ConnectStream>, Message>),
}

pub struct WsSink {
    pub(super) inner: WsSinkInner,
}

impl WsSink {
    pub async fn send(&mut self, msg: WsMessage) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsSinkInner::H1(s) => s.send(msg).await,
            WsSinkInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| Error::new(Kind::Request).with_message(format!("ws send: {e}")))
    }

    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsSinkInner::H1(s) => s.close().await,
            WsSinkInner::H2(s) => s.close().await,
        }
        .map_err(|e| Error::new(Kind::Request).with_message(format!("ws close: {e}")))
    }
}

pub(super) enum WsStreamInner {
    H1(SplitStream<WebSocketStream<TlsIo>>),
    H2(SplitStream<WebSocketStream<H2ConnectStream>>),
}

pub struct WsStream {
    pub(super) inner: WsStreamInner,
}

impl WsStream {
    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsStreamInner::H1(s) => s.next().await,
            WsStreamInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(WsMessage::wire(msg))),
            Some(Err(e)) => Err(Error::new(Kind::Request).with_message(format!("ws recv: {e}"))),
            None => Ok(None),
        }
    }
}
