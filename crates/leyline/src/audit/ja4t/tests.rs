use super::*;
use crate::profile::Platform;

#[test]
fn ja4t_linux() {
    let tcp = TcpProfile {
        window_size: 29200,
        ..Platform::Linux.tcp_profile()
    };
    assert_eq!(compute_ja4t(&tcp), "29200_2-4-8-1-3_1460_7");
}

#[test]
fn ja4t_windows() {
    let fp = compute_ja4t(&Platform::Windows.tcp_profile());
    assert_eq!(fp, "64240_2-1-3-1-1-4_1460_8");
}
