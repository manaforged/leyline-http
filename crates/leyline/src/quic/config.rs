use std::time::Duration;

use crate::{Error, Kind};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H3Config {
    pub initial_max_data: u64,
    pub initial_max_stream_data_bidi_local: u64,
    pub initial_max_stream_data_bidi_remote: u64,
    pub initial_max_stream_data_uni: u64,
    pub initial_max_streams_bidi: u64,
    pub initial_max_streams_uni: u64,
    pub max_idle_timeout: Duration,
    pub max_udp_payload_size: u16,
    pub active_connection_id_limit: u64,
    pub dcid_length: usize,

    pub qpack_max_table_capacity: u64,
    pub qpack_blocked_streams: u64,
    pub max_field_section_size: u64,

    pub max_response_body_bytes: u64,
}

impl H3Config {
    pub fn for_family(family: &str) -> Result<Self, Error> {
        match family {
            "chromium" => Ok(Self::chrome()),
            "gecko" => Ok(Self::firefox()),
            "webkit" => Ok(Self::safari()),
            other => Err(Error::new(Kind::Config)
                .with_message(format!("no HTTP/3 config for profile family {other:?}"))),
        }
    }

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
