use std::time::Duration;

use crate::profile::BrowserProfile;
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

const MAX_RESPONSE_BODY_BYTES: u64 = 100 * 1024 * 1024;

impl H3Config {
    pub fn from_profile(profile: &BrowserProfile) -> Result<Self, Error> {
        let h3 = profile.h3.as_ref().ok_or_else(|| {
            Error::new(Kind::Config).with_message(format!(
                "profile {} has no [h3] table, so it has no HTTP/3 transport",
                profile.meta.name
            ))
        })?;
        Ok(Self {
            initial_max_data: h3.initial_max_data,
            initial_max_stream_data_bidi_local: h3.initial_max_stream_data_bidi_local,
            initial_max_stream_data_bidi_remote: h3.initial_max_stream_data_bidi_remote,
            initial_max_stream_data_uni: h3.initial_max_stream_data_uni,
            initial_max_streams_bidi: h3.initial_max_streams_bidi,
            initial_max_streams_uni: h3.initial_max_streams_uni,
            max_idle_timeout: Duration::from_secs(h3.max_idle_timeout_secs),
            max_udp_payload_size: h3.max_udp_payload_size,
            active_connection_id_limit: h3.active_connection_id_limit,
            dcid_length: h3.dcid_length,
            qpack_max_table_capacity: h3.qpack_max_table_capacity,
            qpack_blocked_streams: h3.qpack_blocked_streams,
            max_field_section_size: h3.max_field_section_size,
            max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
        })
    }
}
