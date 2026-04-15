//! HTTP/3 + QUIC fingerprint configuration.
//!
//! Defines the transport parameters and H3 SETTINGS that match
//! real browser behavior (Chrome, Firefox, Safari).

use std::time::Duration;

/// HTTP/3 configuration for browser fingerprinting.
#[derive(Debug, Clone)]
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
}

impl H3Config {
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
            qpack_max_table_capacity: 4096,
            qpack_blocked_streams: 10,
            max_field_section_size: 262_144,
        }
    }

    /// Firefox 148 QUIC/H3 configuration.
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
        }
    }
}
