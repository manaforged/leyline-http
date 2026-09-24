use std::sync::Mutex;

use socket2::Socket;

mod platform;

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TcpProfile {
    pub ttl: u32,
    pub mss: u32,
    pub window_size: u32,
    pub df: bool,
    pub window_scale: u32,
    pub no_delay: bool,
}

impl TcpProfile {
    pub const WINDOWS: Self = Self {
        ttl: 128,
        mss: 1460,
        window_size: 64240,
        df: true,
        window_scale: 8,
        no_delay: true,
    };

    pub const MACOS: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 6,
        no_delay: true,
    };

    pub const LINUX: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 7,
        no_delay: true,
    };

    pub const IOS: Self = Self {
        ttl: 64,
        mss: 1460,
        window_size: 65535,
        df: true,
        window_scale: 0,
        no_delay: false,
    };

    pub(crate) fn apply(&self, socket: &Socket, is_v6: bool) {
        if self.ttl > 0 {
            let result = if is_v6 {
                socket.set_unicast_hops_v6(self.ttl)
            } else {
                socket.set_ttl_v4(self.ttl)
            };
            if let Err(e) = result {
                log_once(if is_v6 { "IPV6_UNICAST_HOPS" } else { "IP_TTL" }, &e);
            }
        }

        if self.no_delay
            && let Err(e) = socket.set_tcp_nodelay(true)
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
