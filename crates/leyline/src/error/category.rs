use http::StatusCode;

use super::{Cause, Error};

macro_rules! kinds {
    ($($(#[$doc:meta])* $variant:ident => $label:literal,)*) => {
        const COUNT: usize = [$($label),*].len();

        const LABELS: [&str; COUNT] = [$($label),*];

        /// The category of an `Error`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Kind {
            $($(#[$doc])* $variant,)*
        }

        impl Kind {
            pub(crate) const ALL: [Kind; COUNT] = [$(Self::$variant),*];
        }
    };
}

kinds! {
    /// A time limit expired: `total`, `response_header`, `connect` (DNS, TCP, proxy, TLS, or QUIC handshake), or `read` or `body` during a body read.
    Timeout => "timeout",
    /// Name resolution failed.
    Dns => "dns",
    /// The TCP connect failed, or the socket was refused or unreachable.
    Connect => "connect",
    /// The TLS handshake or certificate verification failed.
    Tls => "tls",
    /// The proxy dial, handshake, authentication, or `CONNECT` failed, or the session rejected an invalid environment proxy.
    Proxy => "proxy",
    /// The proxy could not reach the origin.
    ProxyTarget => "proxy_target",
    /// `error_for_status` rejected the status.
    Status => "status",
    /// A response body passed `max_body_size` or a caller limit.
    BodyLimit => "body_limit",
    /// A body cannot be used: a response body already taken, a request body that cannot be compressed or resent, or a WebSocket protocol violation or oversized message.
    Body => "body",
    /// Decompression failed: bad data, a truncated stream, or more than 4 codings.
    Decode => "decode",
    /// JSON serialization or deserialization failed.
    Json => "json",
    /// An HTTP/2 protocol failure occurred.
    Http2 => "http2",
    /// An HTTP/3 protocol or handshake failure occurred.
    Http3 => "http3",
    /// A redirect could not be followed: a custom policy passed 32 hops, a `Location` scheme other than http or https, or a redirect that must resend a streamed request body.
    Redirect => "redirect",
    /// The URL did not parse.
    Url => "url",
    /// The session, request, TLS, trust store, or proxy configuration is invalid, including an invalid environment proxy at `build()`.
    Config => "config",
    /// The request is malformed at the HTTP level, such as a bad header.
    Request => "request",
    /// Socket I/O failed after the connect, or an HTTP/1.1 response broke the wire format.
    Io => "io",
    /// Too many requests already wait for a slot, past `max_pending_per_host`, `HostLimits::max_pending`, or `HostLimits::max_total_pending`.
    Overloaded => "overloaded",
}

impl Kind {
    /// Returns the lowercase label for the kind, such as `"timeout"`.
    pub fn as_str(self) -> &'static str {
        LABELS[self as usize]
    }

    /// Returns the status a gateway should answer with: 504 for a timeout, 500 for a local fault, 503 when overloaded, 502 otherwise, and `None` for `Status`.
    pub fn gateway_status(self) -> Option<StatusCode> {
        match self {
            Self::Status => None,
            Self::Timeout => Some(StatusCode::GATEWAY_TIMEOUT),
            Self::Overloaded => Some(StatusCode::SERVICE_UNAVAILABLE),
            Self::Url | Self::Config | Self::Request => Some(StatusCode::INTERNAL_SERVER_ERROR),
            _ => Some(StatusCode::BAD_GATEWAY),
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Error {
    /// Returns the error's category.
    pub fn kind(&self) -> Kind {
        if self.is_timeout() {
            return Kind::Timeout;
        }
        if self.is_status() {
            return Kind::Status;
        }
        if self.is_body_limit() {
            return Kind::BodyLimit;
        }
        match self.facts().and_then(Cause::kind) {
            Some(kind) => kind,
            None => self.kind_category(),
        }
    }

    fn kind_category(&self) -> Kind {
        match self.inner.kind {
            kind @ (Kind::Proxy | Kind::Tls | Kind::Connect) => kind,
            _ if self.is_socket_unreachable() => Kind::Connect,
            kind => kind,
        }
    }
}
