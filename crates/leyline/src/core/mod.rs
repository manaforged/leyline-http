mod atomic;
mod attempt;
mod block;

mod body_stream;
mod config;
#[cfg(feature = "html")]
mod crawl;
mod decompress;
mod digest;

mod fetch_site;
mod headers;
pub(crate) mod hsts;
mod into_url;
#[cfg(feature = "multipart")]
pub mod multipart;
mod proxy_pool;
mod request;
pub(crate) mod request_headers;
mod response;

pub(crate) mod retry;
mod robots;
pub(crate) mod saved;
#[cfg(feature = "tower")]
mod service;
pub(crate) mod session;
mod shutdown;
mod stats;
mod tab;
#[cfg(feature = "websocket")]
mod websocket;

pub use attempt::Attempt;
pub use block::Block;
pub(crate) use config::BodyRules;
pub use config::{HostLimits, Proxy, Redirect};
#[cfg(feature = "html")]
pub use crawl::{Crawl, Page, Respect, Scope, SkipReason};

pub(crate) use fetch_site::FetchSite;
pub use into_url::IntoUrl;
pub use request::{Event, Events, RequestBuilder};

#[cfg(feature = "bench-internals")]
pub(crate) use response::parse_links;
pub use response::{Link, Response, relay_headers};
pub use retry::Retry;
pub use robots::{Purpose, Robots, RobotsSource};
pub(crate) use session::Identity;
pub use session::{Protocol, Session, SessionBuilder};
pub use stats::Stats;
pub use tab::Tab;
#[cfg(feature = "websocket")]
pub use websocket::{Message, WebSocket};
