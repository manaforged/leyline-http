use crate::tcp::TcpProfile;

pub fn compute_ja4t(tcp: &TcpProfile) -> String {
    let options = tcp
        .options
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}_{options}_{}_{}",
        tcp.window_size, tcp.mss, tcp.window_scale
    )
}

#[cfg(test)]
mod tests;
