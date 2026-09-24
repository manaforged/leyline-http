use leyline_quiche as quiche;

use crate::profile::BrowserProfile;
use crate::tls::{Resolver, TlsMinVersion, TlsTrustConfig, apply_profile_with_trust};

use crate::quic::config::H3Config;

#[derive(Debug)]
pub struct H3Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub trailers: Vec<(String, String)>,
}

pub(crate) struct EstablishedH3 {
    pub(crate) socket: tokio::net::UdpSocket,
    pub(crate) conn: Box<quiche::Connection>,
    pub(crate) h3: quiche::h3::Connection,
    pub(crate) peer_addr: std::net::SocketAddr,
    pub(crate) local_addr: std::net::SocketAddr,
    pub(crate) max_udp_payload: usize,
    pub(crate) max_response_body_bytes: u64,
    pub(crate) tls: crate::pool::TlsInfo,
}

fn build_quic_config(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    host: &str,
) -> Result<quiche::Config, String> {
    let mut ssl_builder =
        leyline_bssl::ssl::SslContextBuilder::new(leyline_bssl::ssl::SslMethod::tls())
            .map_err(|e| format!("quic ssl ctx: {e}"))?;
    apply_profile_with_trust(&mut ssl_builder, profile, TlsMinVersion::Tls13, trust)
        .map_err(|e| format!("quic ssl ctx: {e}"))?;

    let pins = trust.pinned_leaf_sha256();
    if !pins.is_empty() || cfg!(target_os = "macos") && trust.uses_system_roots() {
        crate::tls::install_verifier_ctx(
            &mut ssl_builder,
            pins,
            Some(host),
            trust.uses_system_roots(),
        );
    }

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

fn build_h3_config(h3_cfg: &H3Config) -> Result<quiche::h3::Config, String> {
    let mut h3_config = quiche::h3::Config::new().map_err(|e| format!("h3 config: {e}"))?;
    h3_config.set_qpack_max_table_capacity(h3_cfg.qpack_max_table_capacity);
    h3_config.set_qpack_blocked_streams(h3_cfg.qpack_blocked_streams);
    h3_config.set_max_field_section_size(h3_cfg.max_field_section_size);
    Ok(h3_config)
}

pub(crate) async fn connect_and_handshake(
    h3_cfg: &H3Config,
    profile: &BrowserProfile,
    trust: &TlsTrustConfig,
    resolver: &dyn Resolver,
    host: &str,
    port: u16,
) -> Result<EstablishedH3, String> {
    validate_connection_id_len(h3_cfg.dcid_length)?;
    let host = crate::util::bare_host(host);

    let mut config = build_quic_config(h3_cfg, profile, trust, host)?;
    let peer_addr = resolve_peer(resolver, host, port).await?;

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

    let mut scid_bytes = vec![0u8; h3_cfg.dcid_length];
    use rand::TryRngCore;
    rand::rngs::OsRng
        .try_fill_bytes(scid_bytes.as_mut_slice())
        .map_err(|e| format!("scid entropy: {e}"))?;
    let scid = quiche::ConnectionId::from_ref(&scid_bytes);

    let mut conn = Box::new(
        quiche::connect(Some(host), &scid, local_addr, peer_addr, &mut config)
            .map_err(|e| format!("quic connect: {e}"))?,
    );

    let h3_config = build_h3_config(h3_cfg)?;

    let mut out = vec![0u8; h3_cfg.max_udp_payload_size as usize];
    let mut buf = vec![0u8; 65_535];
    let mut h3: Option<quiche::h3::Connection> = None;
    let mut iter = 0u32;

    loop {
        iter += 1;
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

        if conn.is_established() && h3.is_none() {
            let h3_conn = quiche::h3::Connection::with_transport(&mut conn, &h3_config)
                .map_err(|e| format!("h3 with_transport: {e}"))?;
            h3 = Some(h3_conn);
            flush_egress(&socket, &mut conn, &mut out).await?;
        }

        if let Some(h3) = h3.take() {
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

async fn resolve_peer(
    resolver: &dyn Resolver,
    host: &str,
    port: u16,
) -> Result<std::net::SocketAddr, String> {
    let addrs = resolver
        .resolve(host, port)
        .await
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

pub(crate) fn check_body_budget(already: usize, n: usize, max: u64) -> Result<(), u64> {
    let new_total = already.saturating_add(n) as u64;
    if new_total > max {
        Err(new_total)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
