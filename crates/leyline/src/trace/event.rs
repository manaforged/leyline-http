use std::fmt;
use std::time::Duration;

use http::StatusCode;
use url::Url;

use super::masked;
use crate::Error;
use crate::profile::Browser;

/// One step of a request, passed to [`Trace::event`](super::Trace::event).
#[non_exhaustive]
pub enum Event<'a> {
    /// A host name resolved to its addresses.
    #[non_exhaustive]
    Dns {
        /// The id of the attempt that resolved the name.
        id: u64,
        /// The host name that was resolved.
        host: &'a str,
        /// The port the request will connect to.
        port: u16,
        /// The number of addresses the name resolved to.
        addrs: usize,
        /// The time since the attempt started.
        elapsed: Duration,
    },
    /// A connection opened, or a pooled one was taken.
    #[non_exhaustive]
    Connect {
        /// The id of the attempt that holds the connection.
        id: u64,
        /// The host the connection goes to.
        host: &'a str,
        /// The port the connection goes to.
        port: u16,
        /// Whether the connection came from the pool.
        reused: bool,
        /// The time since the attempt started.
        elapsed: Duration,
    },
    /// A TLS handshake finished.
    #[non_exhaustive]
    Tls {
        /// The id of the attempt that ran the handshake.
        id: u64,
        /// The host the handshake was made with.
        host: &'a str,
        /// The negotiated TLS version, when the backend reports it.
        version: Option<&'a str>,
        /// The negotiated cipher suite, when the backend reports it.
        cipher: Option<&'a str>,
        /// The negotiated ALPN protocol, when the server chose one.
        alpn: Option<&'a str>,
        /// The time since the attempt started.
        elapsed: Duration,
    },
    /// The request head went out on the wire.
    #[non_exhaustive]
    Sent {
        /// The id of the attempt that sent the request.
        id: u64,
        /// The host the request went to.
        host: &'a str,
        /// The request method.
        method: &'a str,
        /// The request target, with its query string unredacted.
        path: &'a str,
        /// The HTTP version the request used.
        protocol: http::Version,
        /// The time since the attempt started.
        elapsed: Duration,
    },
    /// The response headers arrived.
    #[non_exhaustive]
    Head {
        /// The id of the attempt that got the response.
        id: u64,
        /// The host that answered.
        host: &'a str,
        /// The response status code.
        status: u16,
        /// The HTTP version of the response.
        protocol: http::Version,
        /// The time since the attempt started.
        elapsed: Duration,
        /// The response headers, unredacted.
        headers: &'a http::HeaderMap,
    },
    /// An attempt finished, with its result.
    #[non_exhaustive]
    Done {
        /// The id of the attempt that finished.
        id: u64,
        /// The time since the attempt started.
        elapsed: Duration,
        /// The result of the attempt.
        outcome: Result<(), &'a Error>,
    },
    /// The totals of one request after all redirects and retries.
    #[non_exhaustive]
    Summary {
        /// The id of the request.
        id: u64,
        /// The request method.
        method: &'a str,
        /// The final URL, or the parsed original URL when the request failed.
        url: Option<&'a Url>,
        /// The URL the caller asked for, unredacted.
        original_url: &'a str,
        /// The number of redirects the request followed.
        redirects: usize,
        /// The final response status, when a response arrived.
        status: Option<StatusCode>,
        /// The HTTP version of the final response, when a response arrived.
        version: Option<http::Version>,
        /// The number of attempts the request made.
        attempts: u32,
        /// The time since the request started.
        elapsed: Duration,
        /// The result of the request.
        outcome: Result<(), &'a Error>,
        /// Whether the caller asked for a streamed body.
        streamed: bool,
        /// The label the caller set on the request, if any.
        label: Option<&'a str>,
        /// The proxy the request went through, if any.
        proxy: Option<String>,
        /// The browser profile the request used, if any.
        browser: Option<Browser>,
    },
    /// A streamed response body ended.
    #[non_exhaustive]
    Body {
        /// The id of the request that owns the body.
        id: u64,
        /// The number of body bytes read.
        bytes: u64,
        /// The time since the request started.
        elapsed: Duration,
        /// `Ok(())` when the body completed, `Err(Some(e))` when a read failed, and `Err(None)` when the body was dropped before the end.
        outcome: Result<(), Option<&'a std::io::Error>>,
    },
}

impl fmt::Debug for Event<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dns {
                id,
                host,
                port,
                addrs,
                elapsed,
            } => f
                .debug_struct("Dns")
                .field("id", id)
                .field("host", host)
                .field("port", port)
                .field("addrs", addrs)
                .field("elapsed", elapsed)
                .finish(),
            Self::Connect {
                id,
                host,
                port,
                reused,
                elapsed,
            } => f
                .debug_struct("Connect")
                .field("id", id)
                .field("host", host)
                .field("port", port)
                .field("reused", reused)
                .field("elapsed", elapsed)
                .finish(),
            Self::Tls {
                id,
                host,
                version,
                cipher,
                alpn,
                elapsed,
            } => f
                .debug_struct("Tls")
                .field("id", id)
                .field("host", host)
                .field("version", version)
                .field("cipher", cipher)
                .field("alpn", alpn)
                .field("elapsed", elapsed)
                .finish(),
            Self::Sent {
                id,
                host,
                method,
                path,
                protocol,
                elapsed,
            } => f
                .debug_struct("Sent")
                .field("id", id)
                .field("host", host)
                .field("method", method)
                .field("path", &crate::util::mask_query(path))
                .field("protocol", protocol)
                .field("elapsed", elapsed)
                .finish(),
            Self::Head {
                id,
                host,
                status,
                protocol,
                elapsed,
                headers,
            } => f
                .debug_struct("Head")
                .field("id", id)
                .field("host", host)
                .field("status", status)
                .field("protocol", protocol)
                .field("elapsed", elapsed)
                .field(
                    "headers",
                    &masked(headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
                )
                .finish(),
            Self::Done {
                id,
                elapsed,
                outcome,
            } => f
                .debug_struct("Done")
                .field("id", id)
                .field("elapsed", elapsed)
                .field("outcome", outcome)
                .finish(),
            Self::Summary {
                id,
                method,
                url,
                original_url,
                redirects,
                status,
                version,
                attempts,
                elapsed,
                outcome,
                streamed,
                label,
                proxy,
                browser,
            } => f
                .debug_struct("Summary")
                .field("id", id)
                .field("method", method)
                .field("url", &url.map(|u| crate::util::redact(u.as_str())))
                .field("original_url", &crate::util::redact(original_url))
                .field("redirects", redirects)
                .field("status", status)
                .field("version", version)
                .field("attempts", attempts)
                .field("elapsed", elapsed)
                .field("outcome", outcome)
                .field("streamed", streamed)
                .field("label", label)
                .field("proxy", proxy)
                .field("browser", browser)
                .finish(),
            Self::Body {
                id,
                bytes,
                elapsed,
                outcome,
            } => f
                .debug_struct("Body")
                .field("id", id)
                .field("bytes", bytes)
                .field("elapsed", elapsed)
                .field("outcome", outcome)
                .finish(),
        }
    }
}
