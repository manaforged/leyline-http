//! JA4T TCP fingerprint computation.
//!
//! Format: `{window_size}_{options}_{mss}_{window_scale}`
//! No hashing — plaintext fingerprint from TCP SYN parameters.

/// Compute JA4T from TCP profile parameters.
///
/// This uses the values we configure via `socket2` in the TLS connector,
/// which match the target OS's TCP SYN behavior.
///
/// The TCP option strings below are fixed per-OS templates synced by hand
/// with the socket options `tcp/platform.rs` actually sets — they are not
/// derived from [`crate::tcp::TcpProfile`]. If platform.rs changes its
/// SYN option layout, these templates must follow.
pub fn compute_ja4t(window_size: u32, mss: u16, window_scale: u8, is_windows: bool) -> String {
    // TCP options in the order they appear in the SYN packet.
    // Linux/macOS: MSS, SACK-permitted, Timestamps, NOP, Window-Scale → 2-4-8-1-3
    // Windows: MSS, NOP, Window-Scale, NOP, NOP, SACK-permitted → 2-1-3-1-1-4
    let options = if is_windows {
        "2-1-3-1-1-4"
    } else {
        "2-4-8-1-3"
    };

    format!("{window_size}_{options}_{mss}_{window_scale}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ja4t_linux() {
        let fp = compute_ja4t(29200, 1460, 7, false);
        assert_eq!(fp, "29200_2-4-8-1-3_1460_7");
    }

    #[test]
    fn ja4t_windows() {
        let fp = compute_ja4t(64240, 1460, 8, true);
        assert_eq!(fp, "64240_2-1-3-1-1-4_1460_8");
    }
}
