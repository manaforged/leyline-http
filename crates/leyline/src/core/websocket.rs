use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as WireClose, Role};

use crate::h2::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::Pool;
use crate::pool::h1::H1Io;
use crate::tls::FingerprintConnector;

use crate::error::{Error, Kind, Result};

mod debug;
mod handshake;
mod split;

pub(crate) use handshake::{Handshake, http_equivalent};
use handshake::{
    check_subprotocol, check_upgrade_response, random_sec_ws_key, tungstenite_config, wire_error,
};
use http::header::SEC_WEBSOCKET_PROTOCOL;
use split::{WsSink, WsStream};

/// A WebSocket message that a connection sends or receives.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Message {
    /// A UTF-8 text message.
    Text(String),
    /// A binary message.
    Binary(Vec<u8>),
    /// A ping control frame.
    Ping,
    /// A pong control frame.
    Pong,
    /// A close frame with an optional status code and reason.
    Close(Option<(u16, String)>),
}

impl Message {
    fn wire(msg: WireMessage) -> Self {
        match msg {
            WireMessage::Text(s) => Self::Text(s.as_str().to_owned()),
            WireMessage::Binary(b) => Self::Binary(b.to_vec()),
            WireMessage::Ping(_) => Self::Ping,
            WireMessage::Pong(_) => Self::Pong,
            WireMessage::Close(frame) => {
                Self::Close(frame.map(|f| (f.code.into(), f.reason.as_str().to_owned())))
            }
            WireMessage::Frame(f) => Self::Binary(f.into_payload().to_vec()),
        }
    }

    fn into_wire(self) -> WireMessage {
        match self {
            Self::Text(s) => WireMessage::Text(s.into()),
            Self::Binary(b) => WireMessage::Binary(b.into()),
            Self::Ping => WireMessage::Ping(Vec::new().into()),
            Self::Pong => WireMessage::Pong(Vec::new().into()),
            Self::Close(frame) => WireMessage::Close(frame.map(|(code, reason)| WireClose {
                code: code.into(),
                reason: reason.into(),
            })),
        }
    }
}

type WsIo = Box<dyn H1Io + Sync>;

/// An open WebSocket connection.
pub struct WebSocket {
    inner: WebSocketStream<WsIo>,
    version: http::Version,
    protocol: Option<String>,
    headers: Vec<(String, String)>,
}

impl WebSocket {
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        handshake: &Handshake<'_>,
    ) -> Result<Self> {
        let (host, port) = crate::wire::host_port(handshake.url)?;
        let sec_key = random_sec_ws_key();
        let request = handshake.headers(http::Version::HTTP_11, &sec_key)?;

        let mut stream = connector
            .connect(host, port, handshake.proxy, crate::tls::AlpnPlan::Http11)
            .await
            .map_err(Error::from)?
            .stream;

        let ((status, headers, _), leftover) =
            crate::pool::upgrade_on_stream(&mut stream, handshake.url, &request).await?;
        let headers = crate::wire::string_pairs(&headers);
        check_upgrade_response(status, &headers, &sec_key, handshake.extra_headers)?;

        let protocol =
            crate::util::header_first(&headers, SEC_WEBSOCKET_PROTOCOL.as_str()).map(str::to_owned);
        let ws_stream = WebSocketStream::from_partially_read(
            Box::new(stream) as WsIo,
            leftover,
            Role::Client,
            Some(tungstenite_config(handshake.options)),
        )
        .await;

        Ok(Self {
            inner: ws_stream,
            version: http::Version::HTTP_11,
            protocol,
            headers,
        })
    }

    pub(crate) async fn connect_h2(
        pool: &Arc<Pool>,
        connector: &FingerprintConnector,
        h2_config: &H2Config,
        handshake: &Handshake<'_>,
    ) -> Result<Option<Self>> {
        let (host, port) = crate::wire::host_port(handshake.url)?;
        let pseudo = connect_pseudo(handshake.url, host, port);
        let headers = handshake.headers(http::Version::HTTP_2, &random_sec_ws_key())?;

        let Some(stream) = crate::pool::open_extended_connect(
            pool,
            connector,
            h2_config,
            handshake.url,
            handshake.proxy,
            pseudo,
            headers,
        )
        .await?
        else {
            return Ok(None);
        };

        if stream.status() != 200 {
            return Err(Error::new(Kind::Request).with_message(format!(
                "ws handshake: h2 extended CONNECT returned :status {}",
                stream.status()
            )));
        }

        check_subprotocol(stream.response_headers(), handshake.extra_headers)?;

        let protocol =
            crate::util::header_first(stream.response_headers(), SEC_WEBSOCKET_PROTOCOL.as_str())
                .map(str::to_owned);
        let headers = stream.response_headers().to_vec();

        let ws_stream = WebSocketStream::from_raw_socket(
            Box::new(stream) as WsIo,
            Role::Client,
            Some(tungstenite_config(handshake.options)),
        )
        .await;
        Ok(Some(Self {
            inner: ws_stream,
            version: http::Version::HTTP_2,
            protocol,
            headers,
        }))
    }

    pub(crate) fn header_map(&self) -> http::HeaderMap {
        self.headers
            .iter()
            .filter_map(|(name, value)| {
                Some((
                    http::HeaderName::from_bytes(name.as_bytes()).ok()?,
                    http::HeaderValue::from_str(value).ok()?,
                ))
            })
            .collect()
    }

    /// Sends one message.
    pub async fn send(&mut self, msg: Message) -> Result<()> {
        self.inner
            .send(msg.into_wire())
            .await
            .map_err(|e| wire_error("ws send", e))
    }

    /// Receives the next message, or returns `None` once the connection has ended.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        match self.inner.next().await {
            Some(Ok(msg)) => Ok(Some(Message::wire(msg))),
            Some(Err(e)) => Err(wire_error("ws recv", e)),
            None => Ok(None),
        }
    }

    /// Sends a close frame and closes the connection.
    pub async fn close(&mut self) -> Result<()> {
        self.inner
            .close(None)
            .await
            .map_err(|e| wire_error("ws close", e))
    }

    /// Returns the subprotocol that the server selected.
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    /// Returns the first handshake response header with the given name.
    pub fn header(&self, name: &str) -> Option<&str> {
        crate::util::header_first(&self.headers, name)
    }

    /// Splits the connection into a message sink and a message stream.
    pub fn split(
        self,
    ) -> (
        impl futures_util::Sink<Message, Error = Error> + Send,
        impl futures_util::Stream<Item = Result<Message>> + Send,
    ) {
        let (sink, stream) = self.halves();
        let sink = futures_util::sink::unfold(sink, |mut sink, msg: Message| async move {
            sink.send(msg).await?;
            Ok::<_, Error>(sink)
        });
        let stream = futures_util::stream::unfold(Some(stream), |state| async move {
            let mut stream = state?;
            match stream.recv().await {
                Ok(Some(msg)) => Some((Ok(msg), Some(stream))),
                Ok(None) => None,
                Err(e) => Some((Err(e), None)),
            }
        });
        (sink, stream)
    }

    fn halves(self) -> (WsSink, WsStream) {
        let (tx, rx) = self.inner.split();
        (
            WsSink {
                inner: tx,
                version: self.version,
            },
            WsStream {
                inner: rx,
                version: self.version,
            },
        )
    }
}

fn connect_pseudo(url: &url::Url, host: &str, port: u16) -> PseudoHeaders {
    PseudoHeaders {
        method: HeaderStr::from_static("CONNECT"),
        scheme: HeaderStr::from_static("https"),
        authority: HeaderStr::from(crate::wire::authority(url, host, port).as_ref()),
        path: HeaderStr::from(crate::util::request_target(url)),
        protocol: Some(HeaderStr::from_static("websocket")),
    }
}

#[cfg(test)]
mod tests;
