use std::fmt;

use super::{WebSocket, WsInner, WsSink, WsSinkInner, WsStream, WsStreamInner};

use crate::trace::masked;

impl WsInner {
    fn version(&self) -> http::Version {
        match self {
            Self::H1(_) => http::Version::HTTP_11,
            Self::H2(_) => http::Version::HTTP_2,
        }
    }
}

impl fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSocket")
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
            WsSinkInner::H1(_) => http::Version::HTTP_11,
            WsSinkInner::H2(_) => http::Version::HTTP_2,
        };
        f.debug_struct("WsSink")
            .field("transport", &transport)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for WsStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let transport = match &self.inner {
            WsStreamInner::H1(_) => http::Version::HTTP_11,
            WsStreamInner::H2(_) => http::Version::HTTP_2,
        };
        f.debug_struct("WsStream")
            .field("transport", &transport)
            .finish_non_exhaustive()
    }
}
