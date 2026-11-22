use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Sink;
use futures_util::stream::{SplitSink, SplitStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use super::{Message, WsIo, recv_on, wire_error};
use crate::error::{Error, Result};

pub(super) struct WsSink {
    pub(super) inner: SplitSink<WebSocketStream<WsIo>, WireMessage>,
    pub(super) version: http::Version,
}

impl Sink<Message> for WsSink {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner)
            .poll_ready(cx)
            .map_err(|e| wire_error("ws send", e))
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<()> {
        let wire = item.into_wire()?;
        Pin::new(&mut self.inner)
            .start_send(wire)
            .map_err(|e| wire_error("ws send", e))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner)
            .poll_flush(cx)
            .map_err(|e| wire_error("ws send", e))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.inner)
            .poll_close(cx)
            .map_err(|e| wire_error("ws send", e))
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
