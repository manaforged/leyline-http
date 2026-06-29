//! Concurrent multiplexing HTTP/2 client (driver + handle).
//!
//! A single background task — the "driver" — owns the connection's reader
//! and writer halves plus the stream table. Callers interact through the
//! cloneable [`H2Client`] handle, which fans out `send_request` calls to
//! the driver via an mpsc channel. Concurrent requests run as independent
//! streams on the same TCP connection with no head-of-line blocking.
//!
//! This module is the concurrent heart of the crate; the legacy
//! [`crate::h2::connection::ClientConnection`] API is retained as a thin shell
//! around it for backward compatibility.
//!
//! The implementation is split across:
//! - [`types`] — public request/response body types
//! - [`handle`] — the cloneable [`H2Client`]
//! - [`connect_stream`] — the RFC 8441 extended-CONNECT byte stream
//! - [`driver`] — the driver task, per-stream actor, and flow control

mod connect_stream;
mod driver;
mod handle;
mod types;

pub use connect_stream::H2ConnectStream;
pub use handle::H2Client;
pub use types::{H2ResponseEx, RequestBody, ResponseBody};

pub use driver::DriverTask;
pub(crate) use driver::start;
