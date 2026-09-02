//! Concurrent multiplexing HTTP/2 client (driver + handle).

mod connect_stream;
mod driver;
mod handle;
mod types;

pub use connect_stream::H2ConnectStream;
pub use handle::H2Client;
pub use types::{H2ResponseEx, RequestBody, ResponseBody};

pub use driver::DriverTask;
pub(crate) use driver::{Head, start};
