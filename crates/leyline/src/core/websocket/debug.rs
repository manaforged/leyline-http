use std::fmt;

use super::{WsConnection, WsInner, WsSink, WsSinkInner, WsStream, WsStreamInner};
use crate::HttpVersion;
use crate::trace::masked;

impl WsInner {
    fn version(&self) -> HttpVersion {
        match self {
            Self::H1(_) => HttpVersion::Http1_1,
            Self::H2(_) => HttpVersion::Http2,
        }
    }
}

impl fmt::Debug for WsConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsConnection")
            .field("transport", &self.inner.version())
            .field("protocol", &self.protocol)
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
            )
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for WsSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let transport = match &self.inner {
            WsSinkInner::H1(_) => HttpVersion::Http1_1,
            WsSinkInner::H2(_) => HttpVersion::Http2,
        };
        f.debug_struct("WsSink")
            .field("transport", &transport)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for WsStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let transport = match &self.inner {
            WsStreamInner::H1(_) => HttpVersion::Http1_1,
            WsStreamInner::H2(_) => HttpVersion::Http2,
        };
        f.debug_struct("WsStream")
            .field("transport", &transport)
            .finish_non_exhaustive()
    }
}
