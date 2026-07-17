//! HTTP/3 client connection over QUIC.
//!
//! Built on `leyline-quiche` (our fork of Cloudflare `quiche`) which drives
//! the QUIC handshake via BoringSSL through the `leyline_bssl` crate. The same leyline_bssl
//! BoringSSL that emits our H2 ClientHello also produces the QUIC Initial
//! ClientHello — closing the H2/H3 fingerprint drift by construction.
//!
//! [`connect_and_handshake`] drives a fresh QUIC connection to *established*
//! and returns the live transport parts; the persistent request loop that
//! multiplexes requests over that connection lives in [`crate::quic::pool`].

use std::net::ToSocketAddrs;

use leyline_quiche as quiche;

use crate::profile::BrowserProfile;
use crate::tls::{TlsMinVersion, build_ssl_context};

use crate::quic::config::H3Config;

/// An HTTP/3 response.
#[derive(Debug)]
pub struct H3Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
}

/// The live transport parts of an established QUIC + HTTP/3 connection.
///
/// Returned by [`connect_and_handshake`] once the QUIC tunnel is up and the
/// HTTP/3 control streams are exchanged. The pool's driver task takes
/// ownership and multiplexes request streams over it.
pub(crate) struct EstablishedH3 {
    pub(crate) socket: tokio::net::UdpSocket,
    pub(crate) conn: Box<quiche::Connection>,
    pub(crate) h3: quiche::h3::Connection,
    pub(crate) peer_addr: std::net::SocketAddr,
    pub(crate) local_addr: std::net::SocketAddr,
    /// Max UDP payload — sizes the egress buffer in the driver loop.
    pub(crate) max_udp_payload: usize,
    /// Per-response body cap, enforced by the driver.
    pub(crate) max_response_body_bytes: u64,
    /// Peer certificate + negotiated TLS detail, captured at handshake so the
    /// pool can surface it on every response over this connection.
    pub(crate) tls: crate::pool::TlsInfo,
}

/// Build the QUIC `quiche::Config` for a profile via the shared fingerprint
/// factory, so the QUIC ClientHello carries exactly the same cipher list,
/// curve list, sig-alg list, cert compression, delegated credentials, etc.
/// as the H2 TCP path would. QUIC pins TLS 1.3 per RFC 9001 §4.2.
fn build_quic_config(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
) -> Result<quiche::Config, String> {
    let ssl_builder = build_ssl_context(profile, TlsMinVersion::Tls13)
        .map_err(|e| format!("quic ssl ctx: {e}"))?;

    let mut config =
        quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, ssl_builder)
            .map_err(|e| format!("quic config: {e}"))?;
    config.verify_peer(true);
    config
        .set_application_protos(quiche::h3::APPLICATION_PROTOCOL)
        .map_err(|e| format!("quic alpn: {e}"))?;
    config.set_max_idle_timeout(h3_cfg.max_idle_timeout.as_millis() as u64);
    config.set_max_recv_udp_payload_size(h3_cfg.max_udp_payload_size as usize);
    config.set_max_send_udp_payload_size(h3_cfg.max_udp_payload_size as usize);
    config.set_initial_max_data(h3_cfg.initial_max_data);
    config.set_initial_max_stream_data_bidi_local(h3_cfg.initial_max_stream_data_bidi_local);
    config.set_initial_max_stream_data_bidi_remote(h3_cfg.initial_max_stream_data_bidi_remote);
    config.set_initial_max_stream_data_uni(h3_cfg.initial_max_stream_data_uni);
    config.set_initial_max_streams_bidi(h3_cfg.initial_max_streams_bidi);
    config.set_initial_max_streams_uni(h3_cfg.initial_max_streams_uni);
    config.set_active_connection_id_limit(h3_cfg.active_connection_id_limit);
    config.set_disable_active_migration(true);
    Ok(config)
}

/// Build the HTTP/3 `quiche::h3::Config` (QPACK + field-section limits).
fn build_h3_config(h3_cfg: &H3Config) -> Result<quiche::h3::Config, String> {
    let mut h3_config = quiche::h3::Config::new().map_err(|e| format!("h3 config: {e}"))?;
    h3_config.set_qpack_max_table_capacity(h3_cfg.qpack_max_table_capacity);
    h3_config.set_qpack_blocked_streams(h3_cfg.qpack_blocked_streams);
    h3_config.set_max_field_section_size(h3_cfg.max_field_section_size);
    Ok(h3_config)
}

/// Connect to `host:port` over QUIC and drive the handshake until the HTTP/3
/// control streams are exchanged, returning the live transport parts.
///
/// This is the connect-without-send primitive: it returns once the
/// connection is usable but before any request stream is opened. Racing two
/// of these (QUIC vs TCP+TLS) and sending the request only on the winner is
/// how a true Chrome-style H2/H3 race sends the request exactly once.
pub(crate) async fn connect_and_handshake(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    host: &str,
    port: u16,
) -> Result<EstablishedH3, String> {
    validate_connection_id_len(h3_cfg.dcid_length)?;

    let mut config = build_quic_config(h3_cfg, profile)?;
    let peer_addr = resolve_peer(host, port).await?;

    let bind = match peer_addr {
        std::net::SocketAddr::V4(_) => "0.0.0.0:0",
        std::net::SocketAddr::V6(_) => "[::]:0",
    };

    let socket = tokio::net::UdpSocket::bind(bind)
        .await
        .map_err(|e| format!("udp bind: {e}"))?;
    socket
        .connect(peer_addr)
        .await
        .map_err(|e| format!("udp connect: {e}"))?;
    let local_addr = socket
        .local_addr()
        .map_err(|e| format!("local addr: {e}"))?;

    // Generate the browser-profile-sized initial source connection ID.
    let mut scid_bytes = vec![0u8; h3_cfg.dcid_length];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(scid_bytes.as_mut_slice());
    let scid = quiche::ConnectionId::from_ref(&scid_bytes);

    // Heap-box the quiche `Connection` (~14.7 KB by value) so it does not bloat
    // the handshake future's state machine; `&mut conn` deref-coerces at every
    // quiche call site.
    let mut conn = Box::new(
        quiche::connect(Some(host), &scid, local_addr, peer_addr, &mut config)
            .map_err(|e| format!("quic connect: {e}"))?,
    );

    let h3_config = build_h3_config(h3_cfg)?;

    let mut out = vec![0u8; h3_cfg.max_udp_payload_size as usize];
    let mut buf = vec![0u8; 65_535];
    let mut h3: Option<quiche::h3::Connection> = None;
    // Wall-clock ceiling on the handshake. A peer that keeps the loop making
    // progress without ever establishing (or closing) would otherwise spin
    // until some arbitrary iteration count with no relation to elapsed time.
    // The connection cannot usefully outlive its own idle timeout, so reuse it
    // as the real-time bound. The iteration count below is kept purely as a
    // CPU-spin guard for a peer that floods non-progressing packets: each would
    // return from `recv` immediately, burning CPU without advancing the clock
    // much, so the deadline alone wouldn't bound it.
    let deadline = std::time::Instant::now() + h3_cfg.max_idle_timeout;
    let mut iter = 0u32;

    loop {
        iter += 1;
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "h3 handshake: deadline exceeded ({:?}, iter={iter}): closed={} established={} h3={}",
                h3_cfg.max_idle_timeout,
                conn.is_closed(),
                conn.is_established(),
                h3.is_some()
            ));
        }
        if iter > 10_000 {
            return Err(format!(
                "h3 handshake: CPU-spin guard tripped at {iter} iters: closed={} established={} h3={}",
                conn.is_closed(),
                conn.is_established(),
                h3.is_some()
            ));
        }

        flush_egress(&socket, &mut conn, &mut out).await?;

        if conn.is_closed() {
            return Err(close_reason("h3 handshake", iter, &conn));
        }

        // QUIC tunnel up — bring up the HTTP/3 control streams.
        if conn.is_established() && h3.is_none() {
            let h3_conn = quiche::h3::Connection::with_transport(&mut conn, &h3_config)
                .map_err(|e| format!("h3 with_transport: {e}"))?;
            h3 = Some(h3_conn);
            // Flush the H3 SETTINGS / control-stream packets queued above.
            flush_egress(&socket, &mut conn, &mut out).await?;
        }

        if let Some(h3) = h3.take() {
            // Capture peer cert + cipher now, before `conn` moves into the
            // returned struct (and ultimately the driver task). QUIC is always
            // TLS 1.3 per RFC 9001 §4.2.
            let tls = crate::pool::TlsInfo {
                peer_cert_der: conn.peer_cert().map(<[u8]>::to_vec),
                version: Some("TLSv1.3".to_string()),
                cipher: conn.cipher().map(str::to_string),
            };
            return Ok(EstablishedH3 {
                socket,
                conn,
                h3,
                peer_addr,
                local_addr,
                max_udp_payload: h3_cfg.max_udp_payload_size as usize,
                max_response_body_bytes: h3_cfg.max_response_body_bytes,
                tls,
            });
        }

        let timeout = conn.timeout().unwrap_or(std::time::Duration::from_secs(5));
        match tokio::time::timeout(timeout, socket.recv(&mut buf)).await {
            Ok(Ok(len)) => {
                let recv_info = quiche::RecvInfo {
                    from: peer_addr,
                    to: local_addr,
                };
                conn.recv(&mut buf[..len], recv_info)
                    .map_err(|e| format!("quic recv: {e}"))?;
            }
            Ok(Err(e)) => return Err(format!("udp recv: {e}")),
            Err(_) => conn.on_timeout(),
        }
    }
}

/// Drain every queued QUIC packet to the socket.
pub(crate) async fn flush_egress(
    socket: &tokio::net::UdpSocket,
    conn: &mut quiche::Connection,
    out: &mut [u8],
) -> Result<(), String> {
    loop {
        match conn.send(out) {
            Ok((n, _send_info)) => {
                socket
                    .send(&out[..n])
                    .await
                    .map_err(|e| format!("udp send: {e}"))?;
            }
            Err(quiche::Error::Done) => return Ok(()),
            Err(e) => return Err(format!("quic send: {e}")),
        }
    }
}

/// Format a closed-connection diagnostic carrying peer/local error detail.
pub(crate) fn close_reason(ctx: &str, iter: u32, conn: &quiche::Connection) -> String {
    let peer_err = conn.peer_error().map(|e| {
        format!(
            "code={} reason={:?}",
            e.error_code,
            String::from_utf8_lossy(&e.reason)
        )
    });
    let local_err = conn.local_error().map(|e| {
        format!(
            "code={} reason={:?}",
            e.error_code,
            String::from_utf8_lossy(&e.reason)
        )
    });
    format!(
        "{ctx}: conn closed (iter={iter} established={}) peer_err={peer_err:?} local_err={local_err:?}",
        conn.is_established()
    )
}

/// Resolve the QUIC peer off the async runtime.
///
/// `getaddrinfo` is a blocking libc call (multi-second on failing
/// lookups), so it runs under `spawn_blocking` — the H2 path's
/// `SystemResolver` already does the same. IPv4 is preferred because
/// the UDP socket binds `0.0.0.0` by default, with an IPv6 fallback
/// so IPv6-only hosts resolve rather than hard-erroring (the bind match
/// in `connect_and_handshake` already handles both families).
async fn resolve_peer(host: &str, port: u16) -> Result<std::net::SocketAddr, String> {
    // Bare IPv6 literals need brackets for `to_socket_addrs`.
    let addr_str = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let addrs = tokio::task::spawn_blocking(move || {
        addr_str.to_socket_addrs().map(|i| i.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| format!("dns task: {e}"))?
    .map_err(|e| format!("dns: {e}"))?;
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| "no address resolved".to_string())
}

fn validate_connection_id_len(len: usize) -> Result<(), String> {
    if len == 0 || len > quiche::MAX_CONN_ID_LEN {
        return Err(format!(
            "h3: dcid_length must be 1..={} bytes, got {len}",
            quiche::MAX_CONN_ID_LEN
        ));
    }

    Ok(())
}

/// Check whether adding `n` more bytes to an `already`-sized response
/// body would exceed `max`. Returns `Ok(())` when the new total is
/// within budget and `Err(new_total)` when it would exceed the cap,
/// so the caller can emit a diagnostic carrying the proposed size.
///
/// Kept separate from the H3 recv loop so the cap semantics have a unit-
/// test gate — the recv loop itself needs a live QUIC peer, which is
/// infeasible in cargo test. Any refactor that loses this guard
/// reopens the H3 response-body OOM DoS.
pub(crate) fn check_body_budget(already: usize, n: usize, max: u64) -> Result<(), u64> {
    let new_total = already.saturating_add(n) as u64;
    if new_total > max {
        Err(new_total)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
