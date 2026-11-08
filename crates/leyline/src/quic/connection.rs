use leyline_quiche as quiche;

use crate::net::Resolver;
use crate::tls::FingerprintConnector;
use crate::tls::HelloOptions;
use crate::tls::builder::Transport;
use crate::tls::builder::apply_tls_with_trust;
use crate::tls::trust::Tls;

use crate::quic::H3Config;
use crate::quic::error::QuicError;
use crate::quic::transport::DatagramTransport;
use crate::quic::wire;

const DGRAM_QUEUE_LEN: usize = 16;

pub(crate) const UDP_RECV_BUFFER: usize = 65_535;

pub(crate) struct EstablishedH3 {
    pub(crate) socket: DatagramTransport,
    pub(crate) conn: Box<quiche::Connection>,
    pub(crate) h3: quiche::h3::Connection,
    pub(crate) peer_addr: std::net::SocketAddr,
    pub(crate) local_addr: std::net::SocketAddr,
    pub(crate) max_udp_payload: usize,
    pub(crate) priority_update: bool,
    pub(crate) tls: crate::wire::TlsInfo,
}

impl H3Config {
    fn build_quic_config(&self, trust: &Tls, host: &str) -> Result<quiche::Config, QuicError> {
        let mut ssl_builder =
            leyline_bssl::ssl::SslContextBuilder::new(leyline_bssl::ssl::SslMethod::tls())
                .map_err(|e| QuicError::Config(format!("quic ssl ctx: {e}")))?;
        apply_tls_with_trust(&mut ssl_builder, &self.tls, Transport::Quic, trust)
            .map_err(|e| QuicError::Config(format!("quic ssl ctx: {e}")))?;
        crate::tls::install_verifier_ctx(&mut ssl_builder, trust, Some(host), Transport::Quic);

        let mut config =
            quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, ssl_builder)
                .map_err(|e| QuicError::Config(format!("quic config: {e}")))?;
        config.verify_peer(true);
        config
            .set_application_protos(quiche::h3::APPLICATION_PROTOCOL)
            .map_err(|e| QuicError::Config(format!("quic alpn: {e}")))?;
        let h3 = &self.h3;
        config.set_max_idle_timeout(
            std::time::Duration::from_secs(h3.max_idle_timeout_secs).as_millis() as u64,
        );
        config.set_max_recv_udp_payload_size(usize::from(h3.max_udp_payload_size));
        config.set_max_send_udp_payload_size(usize::from(h3.max_udp_payload_size));
        config.set_initial_max_data(h3.initial_max_data);
        config.set_initial_max_stream_data_bidi_local(h3.initial_max_stream_data_bidi_local);
        config.set_initial_max_stream_data_bidi_remote(h3.initial_max_stream_data_bidi_remote);
        config.set_initial_max_stream_data_uni(h3.initial_max_stream_data_uni);
        config.set_initial_max_streams_bidi(h3.initial_max_streams_bidi);
        config.set_initial_max_streams_uni(h3.initial_max_streams_uni);
        config.set_active_connection_id_limit(h3.active_connection_id_limit);
        config.set_disable_active_migration(true);
        if let Some(ms) = h3.max_ack_delay_ms {
            config.set_max_ack_delay(ms);
        }
        if wire::sends_datagrams(h3) {
            config.enable_dgram(true, DGRAM_QUEUE_LEN, DGRAM_QUEUE_LEN);
        }
        if let Some(plan) = wire::transport_plan(h3)? {
            config.set_transport_params_plan(plan);
        }
        config.set_compatible_versions(&wire::compatible_versions(h3));
        config.set_initial_crypto_split(wire::crypto_split(h3));
        if let Some(size) = h3.initial_datagram_size {
            config.set_initial_datagram_size(usize::from(size));
        }
        if h3.settings.is_some() {
            config.grease(false);
        }
        Ok(config)
    }

    fn build_h3_config(&self) -> Result<quiche::h3::Config, QuicError> {
        let h3 = &self.h3;
        let mut h3_config =
            quiche::h3::Config::new().map_err(|e| QuicError::Config(format!("h3 config: {e}")))?;
        h3_config.set_field_section_limit(h3.header_list_limit());
        match &h3.settings {
            Some(settings) => {
                h3_config.set_settings_plan(wire::settings_plan(settings));
                h3_config
                    .set_control_frames(wire::control_frames(h3.control_grease_frame.as_ref()));
            }
            None => {
                h3_config.set_qpack_max_table_capacity(h3.qpack_max_table_capacity.unwrap_or(0));
                h3_config.set_qpack_blocked_streams(h3.qpack_blocked_streams.unwrap_or(0));
                if let Some(size) = h3.max_field_section_size {
                    h3_config.set_max_field_section_size(size);
                }
            }
        }
        Ok(h3_config)
    }
}

struct Opening {
    socket: DatagramTransport,
    conn: Box<quiche::Connection>,
    peer_addr: std::net::SocketAddr,
    local_addr: std::net::SocketAddr,
}

pub(crate) async fn connect_and_handshake(
    h3_cfg: &H3Config,
    trust: &Tls,
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: Option<&crate::net::ProxyUrl>,
) -> Result<EstablishedH3, QuicError> {
    let host = crate::util::bare_host(host);
    let mut opening = open(h3_cfg, trust, connector, host, port, proxy).await?;
    let h3 = handshake(h3_cfg, &mut opening).await?;
    let Opening {
        socket,
        conn,
        peer_addr,
        local_addr,
    } = opening;
    let tls = crate::wire::TlsInfo::from(crate::wire::TlsFacts {
        peer_cert_der: conn.peer_cert().map(Into::into),
        version: Some("TLSv1.3".into()),
        cipher: conn.cipher().map(Into::into),
    });
    Ok(EstablishedH3 {
        socket,
        conn,
        h3,
        peer_addr,
        local_addr,
        max_udp_payload: usize::from(h3_cfg.h3.max_udp_payload_size),
        priority_update: h3_cfg.h3.priority_update,
        tls,
    })
}

fn connection_ids(h3_cfg: &H3Config) -> Result<(Vec<u8>, Vec<u8>), QuicError> {
    let dcid_len = wire::connection_id_len(&h3_cfg.h3.dcid_length);
    validate_connection_id_len(dcid_len)?;
    let scid_len = h3_cfg.h3.scid_length.unwrap_or(dcid_len);
    if scid_len > quiche::MAX_CONN_ID_LEN {
        return Err(QuicError::Config(format!(
            "h3: scid_length must be 0..={} bytes, got {scid_len}",
            quiche::MAX_CONN_ID_LEN
        )));
    }
    Ok((
        crate::util::random_bytes(scid_len),
        crate::util::random_bytes(dcid_len),
    ))
}

async fn open(
    h3_cfg: &H3Config,
    trust: &Tls,
    connector: &FingerprintConnector,
    host: &str,
    port: u16,
    proxy: Option<&crate::net::ProxyUrl>,
) -> Result<Opening, QuicError> {
    let (scid_bytes, dcid_bytes) = connection_ids(h3_cfg)?;
    let mut config = h3_cfg.build_quic_config(trust, host)?;
    let hello = HelloOptions::from_tls(&h3_cfg.tls)
        .map_err(|e| QuicError::Config(format!("quic hello: {e}")))?;
    let (socket, peer_addr) = DatagramTransport::open(connector, host, port, proxy).await?;
    let local_addr = socket
        .local_addr()
        .map_err(|e| QuicError::Socket(format!("local addr: {e}")))?;
    let scid = quiche::ConnectionId::from_ref(&scid_bytes);
    let dcid = quiche::ConnectionId::from_ref(&dcid_bytes);
    let server_name = host.parse::<std::net::IpAddr>().is_err().then_some(host);
    let mut conn = Box::new(
        quiche::connect_with_dcid(
            server_name,
            &scid,
            &dcid,
            local_addr,
            peer_addr,
            &mut config,
        )
        .map_err(|e| QuicError::Protocol(format!("quic connect: {e}")))?,
    );
    hello
        .apply(conn.ssl_mut(), true)
        .map_err(|e| QuicError::Config(format!("quic hello: {e}")))?;
    Ok(Opening {
        socket,
        conn,
        peer_addr,
        local_addr,
    })
}

async fn handshake(
    h3_cfg: &H3Config,
    opening: &mut Opening,
) -> Result<quiche::h3::Connection, QuicError> {
    let h3_config = h3_cfg.build_h3_config()?;
    let Opening {
        socket,
        conn,
        peer_addr,
        local_addr,
    } = opening;
    let mut out = vec![0u8; usize::from(h3_cfg.h3.max_udp_payload_size)];
    let mut buf = vec![0u8; UDP_RECV_BUFFER];
    let mut iter = 0u32;
    loop {
        iter += 1;
        if iter > 10_000 {
            return Err(QuicError::Protocol(format!(
                "h3 handshake: CPU-spin guard tripped at {iter} iters: closed={} established={} h3=false",
                conn.is_closed(),
                conn.is_established(),
            )));
        }
        flush_egress(socket, conn, &mut out).await?;
        if conn.is_closed() {
            return Err(QuicError::Protocol(close_reason(
                "h3 handshake",
                iter,
                conn,
            )));
        }
        if conn.is_established() {
            let h3 = quiche::h3::Connection::with_transport(conn, &h3_config)
                .map_err(|e| QuicError::Protocol(format!("h3 with_transport: {e}")))?;
            flush_egress(socket, conn, &mut out).await?;
            return Ok(h3);
        }
        let timeout = conn.timeout().unwrap_or(super::TIMER_FALLBACK);
        match tokio::time::timeout(timeout, socket.recv(&mut buf)).await {
            Ok(Ok(len)) => {
                let recv_info = quiche::RecvInfo {
                    from: *peer_addr,
                    to: *local_addr,
                };
                conn.recv(&mut buf[..len], recv_info)
                    .map_err(|e| QuicError::Protocol(format!("quic recv: {e}")))?;
            }
            Ok(Err(e)) => return Err(QuicError::Socket(format!("udp recv: {e}"))),
            Err(_) => conn.on_timeout(),
        }
    }
}

pub(crate) async fn flush_egress(
    socket: &DatagramTransport,
    conn: &mut quiche::Connection,
    out: &mut [u8],
) -> Result<(), QuicError> {
    loop {
        match conn.send(out) {
            Ok((n, _send_info)) => {
                socket
                    .send(&out[..n])
                    .await
                    .map_err(|e| QuicError::Socket(format!("udp send: {e}")))?;
            }
            Err(quiche::Error::Done) => return Ok(()),
            Err(e) => return Err(QuicError::Protocol(format!("quic send: {e}"))),
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

pub(super) async fn resolve_peer(
    resolver: &dyn Resolver,
    host: &str,
    port: u16,
) -> Result<std::net::SocketAddr, QuicError> {
    let addrs = resolver
        .resolve(host, port)
        .await
        .map_err(|e| QuicError::Dns(format!("dns: {e}")))?;
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| QuicError::Dns("no address resolved".to_string()))
}

fn validate_connection_id_len(len: usize) -> Result<(), QuicError> {
    if len == 0 || len > quiche::MAX_CONN_ID_LEN {
        return Err(QuicError::Config(format!(
            "h3: dcid_length must be 1..={} bytes, got {len}",
            quiche::MAX_CONN_ID_LEN
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
