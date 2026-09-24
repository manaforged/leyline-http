use std::sync::Mutex;

use serde::Deserialize;
use socket2::Socket;

mod platform;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct TcpProfile {
    pub ttl: u32,
    pub mss: u32,
    pub window_size: u32,
    pub df: bool,
    pub window_scale: u32,
    pub no_delay: bool,
    pub options: Vec<u8>,
}

impl TcpProfile {
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
