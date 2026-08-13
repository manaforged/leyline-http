//! Session, transport, request builder, and response types.

mod body;
mod body_stream;
mod config;
mod digest;
mod error;
mod header_str;
mod headers;
#[cfg(feature = "multipart")]
pub mod multipart;
mod request;
mod response;
pub(crate) mod retry;
#[cfg(feature = "tower")]
mod service;
pub(crate) mod session;
mod standalone;
mod transport;
#[cfg(feature = "websocket")]
mod websocket;

pub use body::Body;
pub use body_stream::BodyStream;
pub use config::{
    CompressionConfig, DnsConfig, NoProxy, PoolConfig, ProxyConfig, ProxyRule, ProxyUrl,
    RedirectAction, RedirectAttempt, RedirectPolicy, SocketConfig, TimeoutConfig, WebSocketConfig,
};
pub use digest::DigestAuth;
pub use error::{Error, Result};
pub(crate) use header_str::HeaderStr;
pub use headers::HeaderList;
pub use request::{ContentEncoding, IntoParamPair, RequestBuilder};
pub use response::{HttpVersion, Response, ResponseTiming};
pub use retry::{RetryPolicy, RetryTrigger};
#[cfg(feature = "tower")]
pub use service::LeylineService;
#[cfg(feature = "websocket")]
pub use session::WebSocketBuilder;
pub use session::{ProtocolPolicy, Session, SessionBuilder};
pub use standalone::Request;
#[cfg(feature = "websocket")]
pub use websocket::{WsConnection, WsSink, WsStream};
