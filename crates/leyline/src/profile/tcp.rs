use serde::Deserialize;

/// TCP socket settings applied to each connection to match a client's network fingerprint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TcpProfile {
    /// The IPv4 TTL or IPv6 hop limit; 0 leaves the OS default.
    pub ttl: u32,
    /// The TCP maximum segment size in bytes.
    pub mss: u32,
    /// The TCP receive window size in bytes.
    pub window_size: u32,
    /// Whether to set the IP Don't Fragment bit.
    pub df: bool,
    /// The TCP window scale factor.
    pub window_scale: u32,
    /// Whether to set `TCP_NODELAY` on the socket.
    pub no_delay: bool,
    /// The TCP option kinds in the order the client sends them.
    pub options: Vec<u8>,
}
