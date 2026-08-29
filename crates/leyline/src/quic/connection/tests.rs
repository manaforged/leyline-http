use super::{check_body_budget, resolve_peer, validate_connection_id_len};

#[tokio::test]
async fn resolve_peer_prefers_ipv4_for_localhost() {
    let addr = resolve_peer("localhost", 443)
        .await
        .expect("resolve localhost");
    // When both families are published, IPv4 must win because the
    // UDP socket binds 0.0.0.0 by default.
    assert!(addr.is_ipv4(), "got {addr}");
}

#[tokio::test]
async fn resolve_peer_falls_back_on_ipv6_only_hosts() {
    // A bare IPv6 literal resolves to exactly one V6 addr — the
    // old inline code errored with "no IPv4 address resolved".
    let addr = resolve_peer("::1", 443).await.expect("resolve ::1");
    assert!(addr.is_ipv6(), "got {addr}");
}

#[test]
fn validates_profile_connection_id_lengths() {
    assert!(validate_connection_id_len(8).is_ok());
    assert!(validate_connection_id_len(0).is_err());
    assert!(validate_connection_id_len(leyline_quiche::MAX_CONN_ID_LEN + 1).is_err());
}

// ---- H3 response-body cap: these unit tests cover the
// body-budget arithmetic directly. If the H3 recv loop stops
// calling check_body_budget, or if the helper ever returns Ok
// for an overflow case, the OOM DoS protection is silently
// disabled. ----

#[test]
fn body_budget_allows_zero_chunks() {
    assert!(check_body_budget(0, 0, 1024).is_ok());
    assert!(check_body_budget(1024, 0, 1024).is_ok());
}

#[test]
fn body_budget_allows_exactly_max() {
    assert!(check_body_budget(0, 1024, 1024).is_ok());
    assert!(check_body_budget(512, 512, 1024).is_ok());
}

#[test]
fn body_budget_rejects_past_max_by_one_byte() {
    let err = check_body_budget(1024, 1, 1024).unwrap_err();
    assert_eq!(err, 1025);
}

#[test]
fn body_budget_rejects_large_chunk_past_cap() {
    let err = check_body_budget(0, 100 * 1024 * 1024 + 1, 100 * 1024 * 1024).unwrap_err();
    assert_eq!(err, (100 * 1024 * 1024 + 1) as u64);
}

#[test]
fn body_budget_saturates_on_usize_add_overflow() {
    // A malicious peer feeding chunk sizes that would wrap usize
    // must not sneak past the cap. saturating_add pins to
    // usize::MAX which converts to u64::MAX on 64-bit targets,
    // guaranteeing the comparison still rejects.
    let err = check_body_budget(usize::MAX, 1, 100 * 1024 * 1024).unwrap_err();
    assert_eq!(err, usize::MAX as u64);
}
