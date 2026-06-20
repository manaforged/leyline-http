//! HTTP/3 client connection over QUIC.
//!
//! Built on `leyline-quiche` (our fork of Cloudflare `quiche`) which drives
//! the QUIC handshake via BoringSSL through the `btls` crate. The same btls
//! BoringSSL that emits our H2 ClientHello also produces the QUIC Initial
//! ClientHello — closing the H2/H3 fingerprint drift by construction.

use std::net::ToSocketAddrs;

use leyline_quiche as quiche;
use quiche::h3::NameValue;

use crate::profile::BrowserProfile;
use crate::tls::{build_ssl_context, TlsMinVersion};

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

/// HTTP/3 connection that sends requests over QUIC.
pub struct H3Connection;

impl H3Connection {
    /// Send an HTTP/3 request using the shared fingerprint-builder so the
    /// emitted ClientHello matches the H2 path byte-for-byte.
    #[tracing::instrument(
        name = "h3.request",
        level = "debug",
        skip(h3_cfg, profile, headers, body),
        fields(http.method = method, host, port, path)
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn request(
        h3_cfg: &H3Config,
        profile: &BrowserProfile,
        method: &str,
        host: &str,
        port: u16,
        path: &str,
        headers: Vec<(String, String)>,
        body: Option<bytes::Bytes>,
    ) -> Result<H3Response, String> {
        validate_connection_id_len(h3_cfg.dcid_length)?;

        // Build the BoringSSL context via the shared fingerprint factory so
        // the QUIC ClientHello carries exactly the same cipher list, curve
        // list, sig-alg list, cert compression, delegated credentials, etc.
        // as the H2 TCP path would for this profile. QUIC pins TLS 1.3 per
        // RFC 9001 §4.2.
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

        // Heap-box the quiche `Connection` (~14.7 KB by value). It lives across
        // every `await` in the drive loop below, so inline it would sit in this
        // request future's state machine for the connection's whole lifetime.
        // Boxing keeps the H3 request future small; `&mut conn` deref-coerces to
        // `&mut quiche::Connection` at every quiche call site, so the loop body
        // is otherwise unchanged.
        let mut conn = Box::new(
            quiche::connect(Some(host), &scid, local_addr, peer_addr, &mut config)
                .map_err(|e| format!("quic connect: {e}"))?,
        );

        let mut h3_config = quiche::h3::Config::new().map_err(|e| format!("h3 config: {e}"))?;
        h3_config.set_qpack_max_table_capacity(h3_cfg.qpack_max_table_capacity);
        h3_config.set_qpack_blocked_streams(h3_cfg.qpack_blocked_streams);
        h3_config.set_max_field_section_size(h3_cfg.max_field_section_size);

        // Build request headers: pseudo-headers first, then caller's.
        let uri_path = if path.is_empty() { "/" } else { path };
        let mut h3_headers: Vec<quiche::h3::Header> = Vec::with_capacity(4 + headers.len());
        h3_headers.push(quiche::h3::Header::new(b":method", method.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":scheme", b"https"));
        h3_headers.push(quiche::h3::Header::new(b":authority", host.as_bytes()));
        h3_headers.push(quiche::h3::Header::new(b":path", uri_path.as_bytes()));
        for (k, v) in &headers {
            h3_headers.push(quiche::h3::Header::new(k.as_bytes(), v.as_bytes()));
        }

        let body = body.unwrap_or_default();
        let mut h3_conn: Option<quiche::h3::Connection> = None;
        let mut req_sent = false;
        let mut req_stream_id: Option<u64> = None;
        let mut body_offset = 0usize;
        let mut resp_status: u16 = 0;
        let mut resp_headers: Vec<(String, String)> = Vec::new();
        let mut resp_body: Vec<u8> = Vec::new();

        let mut out = vec![0u8; h3_cfg.max_udp_payload_size as usize];
        // Heap-allocated receive buffer. As a `[0u8; 65_535]` stack array this
        // was the single largest term in the whole request future — 64 KB held
        // by value across `socket.recv(&mut buf).await` — and it propagated all
        // the way up to size every leyline request's per-request allocation
        // (even pure-H2 traffic, via the H3 match arm). A heap `Vec` is a 24-byte
        // handle in the future; `&mut buf` / `&buf[..n]` slice exactly as before.
        let mut buf = vec![0u8; 65_535];
        let mut iter = 0u32;

        // Drive the handshake + request loop.
        loop {
            iter += 1;
            if iter > 200 {
                return Err(format!(
                    "h3: giving up after 200 iters: closed={} established={} h3={}",
                    conn.is_closed(),
                    conn.is_established(),
                    h3_conn.is_some()
                ));
            }
            // 1. Flush any outgoing QUIC packets.
            loop {
                match conn.send(&mut out) {
                    Ok((n, _send_info)) => {
                        socket
                            .send(&out[..n])
                            .await
                            .map_err(|e| format!("udp send: {e}"))?;
                    }
                    Err(quiche::Error::Done) => break,
                    Err(e) => return Err(format!("quic send: {e}")),
                }
            }

            if conn.is_closed() {
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
                return Err(format!(
                    "h3: conn closed (iter={iter} established={}) peer_err={peer_err:?} local_err={local_err:?}",
                    conn.is_established()
                ));
            }

            // 2. Create H3 conn + send request once the QUIC tunnel is up.
            if conn.is_established() && h3_conn.is_none() {
                let h3 = quiche::h3::Connection::with_transport(&mut conn, &h3_config)
                    .map_err(|e| format!("h3 with_transport: {e}"))?;
                h3_conn = Some(h3);
            }
            if let Some(h3) = h3_conn.as_mut() {
                if !req_sent {
                    let stream_id = h3
                        .send_request(&mut conn, &h3_headers, body.is_empty())
                        .map_err(|e| format!("h3 send_request: {e}"))?;
                    req_stream_id = Some(stream_id);
                    req_sent = true;
                    // Loop around to flush the newly-queued packets before
                    // waiting for a response.
                    continue;
                }

                if let Some(stream_id) = req_stream_id {
                    if body_offset < body.len() {
                        match h3.send_body(&mut conn, stream_id, &body[body_offset..], true) {
                            Ok(written) => {
                                body_offset += written;
                                if written > 0 {
                                    // Loop around to flush queued DATA frames.
                                    continue;
                                }
                            }
                            Err(quiche::h3::Error::Done)
                            | Err(quiche::h3::Error::StreamBlocked) => {}
                            Err(e) => return Err(format!("h3 send_body: {e}")),
                        }
                    }
                }
            }

            // 3. Wait for the next incoming datagram (bounded by quiche's
            //    own timeout, if any).
            let timeout = conn.timeout().unwrap_or(std::time::Duration::from_secs(5));
            let recv = tokio::time::timeout(timeout, socket.recv(&mut buf)).await;

            match recv {
                Ok(Ok(len)) => {
                    let recv_info = quiche::RecvInfo {
                        from: peer_addr,
                        to: local_addr,
                    };
                    conn.recv(&mut buf[..len], recv_info)
                        .map_err(|e| format!("quic recv: {e}"))?;
                }
                Ok(Err(e)) => return Err(format!("udp recv: {e}")),
                Err(_) => {
                    // Quiche timer elapsed — let it move its state.
                    conn.on_timeout();
                }
            }

            // 4. Drain H3 events.
            let mut request_finished = false;
            if let Some(h3) = h3_conn.as_mut() {
                loop {
                    match h3.poll(&mut conn) {
                        Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                            if Some(stream_id) != req_stream_id {
                                continue;
                            }
                            for h in &list {
                                let name = String::from_utf8_lossy(h.name()).to_string();
                                let value = String::from_utf8_lossy(h.value()).to_string();
                                if name == ":status" {
                                    resp_status = value.parse().unwrap_or(0);
                                } else {
                                    resp_headers.push((name, value));
                                }
                            }
                        }
                        Ok((stream_id, quiche::h3::Event::Data)) => {
                            if Some(stream_id) != req_stream_id {
                                continue;
                            }
                            // Bound the response body. Without a cap a
                            // malicious origin can stream gigabytes over
                            // an unbounded QUIC flow-control window and
                            // OOM the client — the H2 and H1 paths both
                            // enforce equivalent limits.
                            while let Ok(n) = h3.recv_body(&mut conn, stream_id, &mut buf) {
                                if let Err(new_len) = check_body_budget(
                                    resp_body.len(),
                                    n,
                                    h3_cfg.max_response_body_bytes,
                                ) {
                                    // Close the stream politely and tear
                                    // down the connection; we can't keep
                                    // reading without exceeding the cap.
                                    let _ = conn.stream_shutdown(
                                        stream_id,
                                        quiche::Shutdown::Read,
                                        quiche::h3::WireErrorCode::ExcessiveLoad as u64,
                                    );
                                    return Err(format!(
                                        "h3: response body exceeded max_response_body_bytes ({} > {})",
                                        new_len, h3_cfg.max_response_body_bytes
                                    ));
                                }
                                resp_body.extend_from_slice(&buf[..n]);
                            }
                        }
                        Ok((stream_id, quiche::h3::Event::Finished)) => {
                            if Some(stream_id) != req_stream_id {
                                continue;
                            }
                            request_finished = true;
                            break;
                        }
                        Ok((stream_id, quiche::h3::Event::Reset(e))) => {
                            if Some(stream_id) != req_stream_id {
                                continue;
                            }
                            return Err(format!("h3 stream reset: {e}"));
                        }
                        Ok((_, quiche::h3::Event::GoAway))
                        | Ok((_, quiche::h3::Event::PriorityUpdate)) => {}
                        Err(quiche::h3::Error::Done) => break,
                        Err(e) => return Err(format!("h3 poll: {e}")),
                    }
                }
            }

            if request_finished {
                // Tell the peer we're done and drain the final packets.
                let _ = conn.close(true, 0x100, b"done");
                // Flush close packets.
                while let Ok((n, _)) = conn.send(&mut out) {
                    let _ = socket.send(&out[..n]).await;
                }
                break;
            }
        }

        if resp_status == 0 {
            return Err("h3: no response received".into());
        }

        Ok(H3Response {
            status: resp_status,
            headers: resp_headers,
            body: resp_body,
        })
    }
}

/// Resolve the QUIC peer off the async runtime.
///
/// `getaddrinfo` is a blocking libc call (multi-second on failing
/// lookups), so it runs under `spawn_blocking` — the H2 path's
/// `SystemResolver` already does the same. IPv4 is preferred because
/// the UDP socket binds `0.0.0.0` by default, with an IPv6 fallback
/// so IPv6-only hosts resolve rather than hard-erroring (the bind match
/// in `request` already handles both families).
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
