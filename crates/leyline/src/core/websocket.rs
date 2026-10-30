use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WireMessage;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as WireClose, Role};

use crate::core::headers::reorder;
use crate::h2::client::H2ConnectStream;
use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::Pool;
use crate::profile::preset::HeaderPair;
use crate::tls::{FingerprintConnector, TlsIo};

use crate::core::config::WebSocketOptions;
use crate::core::error::{Cause, Error, Result};

mod debug;
mod handshake;
mod split;

use handshake::{
    H2_NO_CONNECT_PROTOCOL, check_subprotocol, check_upgrade_response, handshake_target,
    is_reserved_ws_header, overlay_headers, random_sec_ws_key, response_header, tungstenite_config,
    wire_error, ws_header_pair,
};
use http::header::SEC_WEBSOCKET_PROTOCOL;
use split::{WsSink, WsSinkInner, WsStream, WsStreamInner};

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

enum WsInner {
    H1(WebSocketStream<TlsIo>),
    H2(WebSocketStream<H2ConnectStream>),
}

/// An open WebSocket connection.
pub struct WebSocket {
    inner: WsInner,
    protocol: Option<String>,
    headers: Vec<(String, String)>,
}

impl WebSocket {
    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub(crate) async fn connect_h1(
        connector: &FingerprintConnector,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
        header_order: Option<&[String]>,
        ws_config: &WebSocketOptions,
    ) -> Result<Self> {
        let (parsed, host, port) = handshake_target(url)?;

        let mut stream = connector
            .connect_h1(&host, port, proxy)
            .await
            .map_err(Error::from)?
            .stream;

        let sec_key = random_sec_ws_key();
        let mut request: Vec<HeaderPair> = vec![
            ("Connection".into(), "Upgrade".into()),
            ("Upgrade".into(), "websocket".into()),
            ("User-Agent".into(), user_agent.to_owned().into()),
            ("Origin".into(), origin.to_owned().into()),
            ("Sec-WebSocket-Version".into(), "13".into()),
            ("Sec-WebSocket-Key".into(), sec_key.clone().into()),
        ];
        overlay_headers(&mut request, extra_headers);
        if let Some(order) = header_order {
            reorder(&mut request, order);
        }
        let request = request
            .into_iter()
            .map(|(n, v)| (n.into_owned(), v.into_owned()))
            .collect();

        let ((status, headers, _), leftover) =
            crate::pool::upgrade_on_stream(&mut stream, &parsed, request)
                .await
                .map_err(crate::core::transport::h1_error_to_core)?;
        check_upgrade_response(status, &headers, &sec_key, extra_headers)?;

        let protocol =
            response_header(&headers, SEC_WEBSOCKET_PROTOCOL.as_str()).map(str::to_owned);
        let ws_stream = WebSocketStream::from_partially_read(
            stream,
            leftover,
            Role::Client,
            Some(tungstenite_config(ws_config)),
        )
        .await;

        Ok(Self {
            inner: WsInner::H1(ws_stream),
            protocol,
            headers,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "flat per-request wire fields across one internal call path"
    )]
    pub(crate) async fn connect_h2(
        pool: &Arc<Pool>,
        connector: &FingerprintConnector,
        h2_config: &H2Config,
        url: &str,
        proxy: Option<&str>,
        user_agent: &str,
        origin: &str,
        extra_headers: &[(String, String)],
        ws_config: &WebSocketOptions,
    ) -> Result<Self> {
        let (parsed, host, port) = handshake_target(url)?;
        let path = crate::util::request_target(&parsed);
        let authority = crate::util::authority(&parsed, &host, port);

        let (h2_client, _tls) =
            crate::pool::checkout_handle(pool, connector, h2_config, &host, port, proxy).await?;

        if !h2_client.peer_enables_connect_protocol() {
            return Err(Error::new(Cause::Request).with_message(H2_NO_CONNECT_PROTOCOL));
        }

        for (name, value) in extra_headers
            .iter()
            .filter(|(name, _)| !is_reserved_ws_header(name))
        {
            drop(ws_header_pair(name, value)?);
        }
        let sec_key = random_sec_ws_key();
        let mut headers: Vec<HeaderPair> = vec![
            ("sec-websocket-version".into(), "13".into()),
            ("sec-websocket-key".into(), sec_key.into()),
            ("user-agent".into(), user_agent.to_owned().into()),
            ("origin".into(), origin.to_owned().into()),
        ];
        overlay_headers(&mut headers, extra_headers);

        let pseudo = PseudoHeaders {
            method: HeaderStr::from_static("CONNECT"),
            scheme: HeaderStr::from_static("https"),
            authority: HeaderStr::from(authority),
            path: HeaderStr::from(path),
            protocol: Some(HeaderStr::from_static("websocket")),
        };

        let stream = h2_client
            .open_extended_connect(
                pseudo,
                headers
                    .into_iter()
                    .map(|(name, value)| {
                        (std::borrow::Cow::Owned(name.to_ascii_lowercase()), value)
                    })
                    .collect(),
            )
            .await
            .map_err(|e| Error::new(Cause::Request).with_message(format!("h2 ws: {e}")))?;

        if stream.status() != 200 {
            return Err(Error::new(Cause::Request).with_message(format!(
                "ws handshake: h2 extended CONNECT returned :status {}",
                stream.status()
            )));
        }

        check_subprotocol(stream.response_headers(), extra_headers)?;

        let protocol = response_header(stream.response_headers(), SEC_WEBSOCKET_PROTOCOL.as_str())
            .map(str::to_owned);
        let headers = stream.response_headers().to_vec();

        let ws_stream = WebSocketStream::from_raw_socket(
            stream,
            Role::Client,
            Some(tungstenite_config(ws_config)),
        )
        .await;
        Ok(Self {
            inner: WsInner::H2(ws_stream),
            protocol,
            headers,
        })
    }

    pub(crate) fn is_h2_fallback_trigger(err: &Error) -> bool {
        crate::core::transport::is_h2_alpn_mismatch(err)
            || err.cause() == Cause::Request
                && err
                    .message()
                    .is_some_and(|s| s.contains(H2_NO_CONNECT_PROTOCOL))
    }

    /// Sends one message.
    pub async fn send(&mut self, msg: Message) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsInner::H1(s) => s.send(msg).await,
            WsInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| wire_error("ws send", e))
    }

    /// Receives the next message, or returns `None` once the connection has ended.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        let next = match &mut self.inner {
            WsInner::H1(s) => s.next().await,
            WsInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(Message::wire(msg))),
            Some(Err(e)) => Err(wire_error("ws recv", e)),
            None => Ok(None),
        }
    }

    /// Sends a close frame and closes the connection.
    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s.close(None).await.map_err(|e| wire_error("ws close", e)),
            WsInner::H2(s) => s.close(None).await.map_err(|e| wire_error("ws close", e)),
        }
    }

    /// Returns the subprotocol that the server selected.
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    /// Returns the first handshake response header with the given name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
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
        match self.inner {
            WsInner::H1(s) => {
                let (tx, rx) = s.split();
                (
                    WsSink {
                        inner: WsSinkInner::H1(tx),
                    },
                    WsStream {
                        inner: WsStreamInner::H1(rx),
                    },
                )
            }
            WsInner::H2(s) => {
                let (tx, rx) = s.split();
                (
                    WsSink {
                        inner: WsSinkInner::H2(tx),
                    },
                    WsStream {
                        inner: WsStreamInner::H2(rx),
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod tests;
