use std::fmt;

use super::{WebSocket, WsSink, WsStream};

use crate::util::masked;

impl fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSocket")
            .field("transport", &self.version)
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
        f.debug_struct("WsSink")
            .field("transport", &self.version)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for WsStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsStream")
            .field("transport", &self.version)
            .finish_non_exhaustive()
    }
}
