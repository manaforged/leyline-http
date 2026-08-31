//! JA4T TCP fingerprint computation.

/// Compute JA4T from the TCP profile this session applies via `setsockopt`.
pub fn compute_ja4t(window_size: u32, mss: u16, window_scale: u8, is_windows: bool) -> String {
    let options = if is_windows {
        "2-1-3-1-1-4"
    } else {
        "2-4-8-1-3"
    };

    format!("{window_size}_{options}_{mss}_{window_scale}")
}

#[cfg(test)]
mod tests;
