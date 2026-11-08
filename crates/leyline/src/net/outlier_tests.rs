use std::net::SocketAddr;
use std::time::Duration;

use super::Outliers;

fn addrs() -> (SocketAddr, SocketAddr, SocketAddr) {
    (
        "192.0.2.1:443".parse().unwrap(),
        "192.0.2.2:443".parse().unwrap(),
        "192.0.2.3:443".parse().unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn five_failures_in_a_row_move_an_address_last_for_thirty_seconds() {
    let (a, b, c) = addrs();
    let outliers = Outliers::default();
    for _ in 0..4 {
        outliers.failed(a);
    }
    assert_eq!(outliers.order(vec![a, b, c]), vec![a, b, c]);
    outliers.failed(a);
    assert_eq!(outliers.order(vec![a, b, c]), vec![b, c, a]);
    tokio::time::advance(Duration::from_secs(29)).await;
    assert_eq!(outliers.order(vec![a, b, c]), vec![b, c, a]);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(outliers.order(vec![a, b, c]), vec![a, b, c]);
}

#[tokio::test(start_paused = true)]
async fn each_ejection_lasts_longer_up_to_five_minutes() {
    let (a, b, _) = addrs();
    let outliers = Outliers::default();
    for ejection in 1..=12u64 {
        for _ in 0..5 {
            outliers.failed(a);
        }
        let expected = Duration::from_secs((30 * ejection).min(300));
        tokio::time::advance(expected - Duration::from_secs(1)).await;
        assert_eq!(
            outliers.order(vec![a, b]),
            vec![b, a],
            "ejection {ejection}"
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            outliers.order(vec![a, b]),
            vec![a, b],
            "ejection {ejection}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_success_clears_the_failures_of_an_address() {
    let (a, b, _) = addrs();
    let outliers = Outliers::default();
    for _ in 0..4 {
        outliers.failed(a);
    }
    outliers.succeeded(a);
    outliers.failed(a);
    assert_eq!(outliers.order(vec![a, b]), vec![a, b]);
}

#[tokio::test(start_paused = true)]
async fn every_address_stays_in_the_order_when_all_are_ejected() {
    let (a, b, c) = addrs();
    let outliers = Outliers::default();
    for addr in [a, b, c] {
        for _ in 0..5 {
            outliers.failed(addr);
        }
    }
    assert_eq!(outliers.order(vec![a, b, c]), vec![a, b, c]);
}
