use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame as WireClose, Role};

use crate::core::headers::reorder;
use crate::h2::client::H2ConnectStream;
use crate::h2::config::H2Config;
use crate::h2::connection::PseudoHeaders;
use crate::header_str::HeaderStr;
use crate::pool::Pool;
use crate::profile::preset::HeaderPair;
use crate::tls::{FingerprintConnector, TlsIo};

use crate::core::WebSocketConfig;
use crate::core::error::{Error, Kind, Result};

mod debug;
mod handshake;
mod split;

use handshake::{
    H2_NO_CONNECT_PROTOCOL, check_subprotocol, check_upgrade_response, handshake_target,
    is_reserved_ws_header, overlay_headers, random_sec_ws_key, response_header, tungstenite_config,
    wire_error, ws_header_pair,
};
use http::header::SEC_WEBSOCKET_PROTOCOL;
pub use split::{WsSink, WsStream};
use split::{WsSinkInner, WsStreamInner};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close(Option<CloseFrame>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CloseFrame {
    pub code: u16,
    pub reason: String,
}

impl CloseFrame {
    pub fn new(code: u16, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl WsMessage {
    fn wire(msg: Message) -> Self {
        match msg {
            Message::Text(s) => Self::Text(s.as_str().to_owned()),
            Message::Binary(b) => Self::Binary(b.to_vec()),
            Message::Ping(_) => Self::Ping,
            Message::Pong(_) => Self::Pong,
            Message::Close(frame) => Self::Close(frame.map(|f| CloseFrame {
                code: f.code.into(),
                reason: f.reason.as_str().to_owned(),
            })),
            Message::Frame(f) => Self::Binary(f.into_payload().to_vec()),
        }
    }

    fn into_wire(self) -> Message {
        match self {
            Self::Text(s) => Message::Text(s.into()),
            Self::Binary(b) => Message::Binary(b.into()),
            Self::Ping => Message::Ping(Vec::new().into()),
            Self::Pong => Message::Pong(Vec::new().into()),
            Self::Close(frame) => Message::Close(frame.map(|f| WireClose {
                code: f.code.into(),
                reason: f.reason.into(),
            })),
        }
    }
}

enum WsInner {
    H1(WebSocketStream<TlsIo>),
    H2(WebSocketStream<H2ConnectStream>),
}

pub struct WsConnection {
    inner: WsInner,
    protocol: Option<String>,
    headers: Vec<(String, String)>,
}

impl WsConnection {
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
        ws_config: &WebSocketConfig,
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
        ws_config: &WebSocketConfig,
    ) -> Result<Self> {
        let (parsed, host, port) = handshake_target(url)?;
        let path = crate::util::request_target(&parsed);
        let authority = crate::util::authority(&parsed, &host, port);

        let (h2_client, _tls) =
            crate::pool::checkout_handle(pool, connector, h2_config, &host, port, proxy).await?;

        if !h2_client.peer_enables_connect_protocol() {
            return Err(Error::new(Kind::Request).with_message(H2_NO_CONNECT_PROTOCOL));
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
            .map_err(|e| Error::new(Kind::Request).with_message(format!("h2 ws: {e}")))?;

        if stream.status() != 200 {
            return Err(Error::new(Kind::Request).with_message(format!(
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
            || err.kind() == Kind::Request
                && err
                    .message()
                    .is_some_and(|s| s.contains(H2_NO_CONNECT_PROTOCOL))
    }

    pub async fn send(&mut self, msg: WsMessage) -> Result<()> {
        let msg = msg.into_wire();
        match &mut self.inner {
            WsInner::H1(s) => s.send(msg).await,
            WsInner::H2(s) => s.send(msg).await,
        }
        .map_err(|e| wire_error("ws send", e))
    }

    pub async fn recv(&mut self) -> Result<Option<WsMessage>> {
        let next = match &mut self.inner {
            WsInner::H1(s) => s.next().await,
            WsInner::H2(s) => s.next().await,
        };
        match next {
            Some(Ok(msg)) => Ok(Some(WsMessage::wire(msg))),
            Some(Err(e)) => Err(wire_error("ws recv", e)),
            None => Ok(None),
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsInner::H1(s) => s.close(None).await.map_err(|e| wire_error("ws close", e)),
            WsInner::H2(s) => s.close(None).await.map_err(|e| wire_error("ws close", e)),
        }
    }

    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn split(self) -> (WsSink, WsStream) {
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
