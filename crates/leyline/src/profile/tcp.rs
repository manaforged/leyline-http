use serde::Deserialize;

/// TCP socket settings applied to each connection to match a client's network fingerprint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TcpProfile {
    pub(crate) ttl: u32,
    pub(crate) mss: u32,
    pub(crate) window_size: u32,
    pub(crate) df: bool,
    pub(crate) window_scale: u32,
    pub(crate) no_delay: bool,
    pub(crate) options: Vec<u8>,
}

impl TcpProfile {
    /// Returns the IPv4 TTL or IPv6 hop limit; 0 leaves the OS default.
    #[must_use]
    pub fn ttl(&self) -> u32 {
        self.ttl
    }

    /// Returns the TCP maximum segment size in bytes.
    #[must_use]
    pub fn mss(&self) -> u32 {
        self.mss
    }

    /// Returns the TCP receive window size in bytes.
    #[must_use]
    pub fn window_size(&self) -> u32 {
        self.window_size
    }

    /// Returns whether the IP Don't Fragment bit is set.
    #[must_use]
    pub fn df(&self) -> bool {
        self.df
    }

    /// Returns the TCP window scale factor.
    #[must_use]
    pub fn window_scale(&self) -> u32 {
        self.window_scale
    }

    /// Returns whether `TCP_NODELAY` is set on the socket.
    #[must_use]
    pub fn no_delay(&self) -> bool {
        self.no_delay
    }

    /// Returns the TCP option kinds in the order the client sends them.
    #[must_use]
    pub fn options(&self) -> &[u8] {
        &self.options
    }
}
