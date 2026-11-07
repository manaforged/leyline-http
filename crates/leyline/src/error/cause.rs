use std::error::Error as StdError;
use std::io;

use super::Kind;

/// A typed error source that reports the facts `Error` classifies by.
pub(crate) trait Cause: StdError + Send + Sync + 'static {
    /// Returns the kind the source decides, if any.
    fn kind(&self) -> Option<Kind> {
        None
    }

    /// Returns true when the source failed while connecting.
    fn is_connect_phase(&self) -> bool {
        false
    }

    /// Returns true when the source is a TLS handshake failure.
    fn is_handshake(&self) -> bool {
        false
    }

    /// Returns true when the connection closed under the request.
    fn is_closed(&self) -> bool {
        false
    }

    /// Returns true when the peer never processed the request.
    fn refused(&self) -> bool {
        false
    }

    /// Returns true when the proxy itself failed.
    fn is_proxy(&self) -> bool {
        false
    }

    /// Returns the proxy's HTTP status or SOCKS5 reply code.
    fn proxy_status(&self) -> Option<u16> {
        None
    }

    /// Returns true for a proxy failure worth retrying.
    fn is_transient(&self) -> bool {
        false
    }

    /// Returns the HTTP/2 error code.
    fn h2_code(&self) -> Option<u32> {
        None
    }

    /// Returns the I/O failure underneath the source.
    fn io_source(&self) -> Option<&io::Error> {
        None
    }

    /// Returns a copy of the source, when it can be copied.
    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        None
    }
}

impl Cause for io::Error {}
