//! HTTP/3 + QUIC fingerprint configuration.
//!
//! Defines the transport parameters and H3 SETTINGS that match
//! real browser behavior (Chrome, Firefox, Safari).

use std::time::Duration;

/// HTTP/3 configuration for browser fingerprinting.
///
/// `#[non_exhaustive]` so a new RFC parameter can be added without
/// breaking consumers that constructed with `..Default::default()` or
/// one of the browser presets. Construct via [`H3Config::chrome`] /
/// [`firefox`](H3Config::firefox) / [`safari`](H3Config::safari) and
/// then mutate individual fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H3Config {
    // ─── QUIC transport parameters ─────────────────────────────
    /// Max data the peer can send (connection-level flow control).
    pub initial_max_data: u64,
    /// Max data on a locally-initiated bidirectional stream.
    pub initial_max_stream_data_bidi_local: u64,
    /// Max data on a remotely-initiated bidirectional stream.
    pub initial_max_stream_data_bidi_remote: u64,
    /// Max data on a unidirectional stream.
    pub initial_max_stream_data_uni: u64,
    /// Max concurrent bidirectional streams.
    pub initial_max_streams_bidi: u64,
    /// Max concurrent unidirectional streams.
    pub initial_max_streams_uni: u64,
    /// Connection idle timeout.
    pub max_idle_timeout: Duration,
    /// Max UDP payload size.
    pub max_udp_payload_size: u16,
    /// Active connection ID limit.
    pub active_connection_id_limit: u64,
    /// Initial DCID length (Chrome uses 8 bytes).
    pub dcid_length: usize,

    // ─── HTTP/3 SETTINGS ───────────────────────────────────────
    /// QPACK max table capacity.
    pub qpack_max_table_capacity: u64,
    /// QPACK blocked streams.
    pub qpack_blocked_streams: u64,
    /// Max field section size (like H2 MAX_HEADER_LIST_SIZE).
    pub max_field_section_size: u64,

    // ─── Safety limits (not wire-visible) ──────────────────────
    /// Hard cap on the response body the H3 client will buffer
    /// before aborting the stream. Mirrors the H2 path's
    /// `max_response_body_bytes` — without it, a malicious origin
    /// can stream gigabytes over an unbounded flow-control window
    /// and OOM the client.
    pub max_response_body_bytes: u64,
}

impl H3Config {
    /// Select the H3 transport config for a profile family (`meta.family` in
    /// the TOML). Unknown or empty families are rejected, not defaulted — a
    /// wrong QUIC transport fingerprint is a soft-block risk. Keys are the
    /// family strings the profiles use (`gecko`, `webkit`), not browser names.
    pub fn for_family(family: &str) -> Result<Self, crate::Error> {
        match family {
            "chromium" => Ok(Self::chrome()),
            "gecko" => Ok(Self::firefox()),
            "webkit" => Ok(Self::safari()),
            other => Err(crate::Error::Config(format!(
                "no HTTP/3 config for profile family {other:?}"
            ))),
        }
    }

    /// Chrome 147 QUIC/H3 configuration.
    pub fn chrome() -> Self {
        Self {
            initial_max_data: 15_728_640,
            initial_max_stream_data_bidi_local: 6_291_456,
            initial_max_stream_data_bidi_remote: 6_291_456,
            initial_max_stream_data_uni: 6_291_456,
            initial_max_streams_bidi: 100,
            initial_max_streams_uni: 100,
            max_idle_timeout: Duration::from_secs(30),
            max_udp_payload_size: 1472,
            active_connection_id_limit: 4,
            dcid_length: 8,
            // Advertised as 0 because leyline-quiche's QPACK decoder
            // (h3/qpack/decoder.rs) is stubbed for the dynamic-table
            // path — its dynamic-table branches return InvalidHeaderValue
            // → QpackDecompressionFailed. Real Chrome advertises 65536/100,
            // but advertising a non-zero capacity requires the fork's
            // decoder to support the dynamic table. RFC 9204 §3.1 says an
            // encoder MUST NOT emit dynamic-table references when the peer's
            // advertised capacity is zero, so advertising 0 forces Google's
            // H3 server to use only static-table refs (which the decoder
            // handles). Cloudflare's server happens to do that anyway —
            // that's why live_h3_cloudflare passes either way and only
            // live_h3_google exposed the gap.
            qpack_max_table_capacity: 0,
            qpack_blocked_streams: 0,
            max_field_section_size: 262_144,
            // 100 MiB default, same as the H1 path's cap.
            max_response_body_bytes: 100 * 1024 * 1024,
        }
    }

    /// Firefox QUIC/H3 configuration.
    pub fn firefox() -> Self {
        Self {
            initial_max_data: 25_165_824,
            initial_max_stream_data_bidi_local: 12_582_912,
            initial_max_stream_data_bidi_remote: 12_582_912,
            initial_max_stream_data_uni: 12_582_912,
            initial_max_streams_bidi: 16,
            initial_max_streams_uni: 16,
            max_idle_timeout: Duration::from_secs(30),
            max_udp_payload_size: 1472,
            active_connection_id_limit: 8,
            dcid_length: 8,
            qpack_max_table_capacity: 65_536,
            qpack_blocked_streams: 20,
            max_field_section_size: 262_144,
            // 100 MiB default, same as the H1 path's cap.
            max_response_body_bytes: 100 * 1024 * 1024,
        }
    }

    /// Safari 18 QUIC/H3 configuration.
    pub fn safari() -> Self {
        Self {
            initial_max_data: 8_388_608,
            initial_max_stream_data_bidi_local: 1_048_576,
            initial_max_stream_data_bidi_remote: 1_048_576,
            initial_max_stream_data_uni: 1_048_576,
            initial_max_streams_bidi: 100,
            initial_max_streams_uni: 100,
            max_idle_timeout: Duration::from_secs(600),
            max_udp_payload_size: 1452,
            active_connection_id_limit: 4,
            dcid_length: 8,
            qpack_max_table_capacity: 4096,
            qpack_blocked_streams: 10,
            max_field_section_size: 262_144,
            // 100 MiB default, same as the H1 path's cap.
            max_response_body_bytes: 100 * 1024 * 1024,
        }
    }
}
