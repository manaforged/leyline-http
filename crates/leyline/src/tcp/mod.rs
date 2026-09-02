//! JA4T TCP fingerprinting via socket2.

use std::sync::Mutex;

use socket2::Socket;

mod platform;

/// TCP/IP stack fingerprint parameters for JA4T matching.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TcpProfile {
    /// IP TTL: 128 for Windows, 64 for macOS/Linux.
    pub ttl: u32,
    /// TCP Max Segment Size: 1460 for standard Ethernet.
    pub mss: u32,
    /// TCP receive window size: 64240 (Windows), 65535 (macOS/Linux).
    pub window_size: u32,
    /// Don't Fragment bit.
    pub df: bool,
    /// TCP window scale factor: 8 (Windows), 6 (macOS), 7 (Linux).
    pub window_scale: u32,
    /// TCP_NODELAY (disables Nagle's algorithm).
    pub no_delay: bool,
}

impl TcpProfile {
    /// TCP profile matching Windows defaults (TTL 128, window scale 8).
    pub const WINDOWS: Self = Self {
        ttl: 128,
        mss: 1460,
        window_size: 64240,
        df: true,
        window_scale: 8,
        no_delay: true,
    };

    /// TCP profile matching macOS defaults (TTL 64, window scale 6).
    pub const MACOS: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 6,
        no_delay: true,
    };

    /// TCP profile matching Linux defaults (TTL 64, window scale 7).
    pub const LINUX: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 7,
        no_delay: true,
    };

    /// TCP profile matching iOS defaults (TTL 64, Nagle enabled).
    pub const IOS: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 0,
        no_delay: false,
    };

    /// Apply this TCP profile to a socket before connect().
    pub(crate) fn apply(&self, socket: &Socket, is_v6: bool) {
        if self.ttl > 0 {
            let result = if is_v6 {
                socket.set_unicast_hops_v6(self.ttl)
            } else {
                socket.set_ttl(self.ttl)
            };
            if let Err(e) = result {
                log_once(if is_v6 { "IPV6_UNICAST_HOPS" } else { "IP_TTL" }, &e);
            }
        }

        if self.window_size > 0
            && let Err(e) = socket.set_recv_buffer_size(self.window_size as usize)
        {
            log_once("SO_RCVBUF", &e);
        }

        if self.no_delay
            && let Err(e) = socket.set_nodelay(true)
        {
            log_once("TCP_NODELAY", &e);
        }

        platform::apply_platform_options(socket, self, is_v6);
    }
}

static LOGGED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn log_once(option: &'static str, err: &std::io::Error) {
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if !logged.contains(&option) {
        logged.push(option);
        tracing::warn!(option, error = %err, "setsockopt failed (non-fatal)");
    }
}

#[cfg(test)]
mod tests;
