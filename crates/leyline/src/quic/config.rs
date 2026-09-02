//! HTTP/3 + QUIC fingerprint configuration.

use std::time::Duration;

use crate::{Error, Kind};

/// HTTP/3 configuration for browser fingerprinting.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H3Config {
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

    /// QPACK max table capacity.
    pub qpack_max_table_capacity: u64,
    /// QPACK blocked streams.
    pub qpack_blocked_streams: u64,
    /// Max field section size (like H2 MAX_HEADER_LIST_SIZE).
    pub max_field_section_size: u64,

    /// Hard cap on the response body the H3 client will buffer before aborting the stream.
    pub max_response_body_bytes: u64,
}

impl H3Config {
    /// Select the H3 transport config for a profile family (`meta.family` in the TOML).
    pub fn for_family(family: &str) -> Result<Self, Error> {
        match family {
            "chromium" => Ok(Self::chrome()),
            "gecko" => Ok(Self::firefox()),
            "webkit" => Ok(Self::safari()),
            other => Err(Error::new(Kind::Config)
                .with_message(format!("no HTTP/3 config for profile family {other:?}"))),
        }
    }

    /// Chrome QUIC transport parameters.
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
            qpack_max_table_capacity: 0,
            qpack_blocked_streams: 0,
            max_field_section_size: 262_144,
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
            qpack_max_table_capacity: 0,
            qpack_blocked_streams: 0,
            max_field_section_size: 262_144,
            max_response_body_bytes: 100 * 1024 * 1024,
        }
    }

    /// Safari QUIC/H3 configuration.
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
            qpack_max_table_capacity: 0,
            qpack_blocked_streams: 0,
            max_field_section_size: 262_144,
            max_response_body_bytes: 100 * 1024 * 1024,
        }
    }
}
