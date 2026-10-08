use std::time::Duration;

use crate::h2::config::{PseudoOrder, parse_pseudo_order};
use crate::profile::{BrowserProfile, H3Profile, TlsProfile};
use crate::{Error, Kind};

const SETTINGS_MAX_FIELD_SECTION_SIZE: u64 = 0x06;

const DEFAULT_PSEUDO_ORDER: [PseudoOrder; 4] = [
    PseudoOrder::Method,
    PseudoOrder::Scheme,
    PseudoOrder::Authority,
    PseudoOrder::Path,
];

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

    pub max_response_body_bytes: u64,

    pub(crate) wire: H3Profile,
    pub(crate) tls: TlsProfile,
    pub(crate) pseudo_order: [PseudoOrder; 4],
    pub(crate) max_header_list_bytes: u64,
}

impl H3Config {
    pub fn from_profile(profile: &BrowserProfile) -> Result<Self, Error> {
        let h3 = profile.h3.as_ref().ok_or_else(|| {
            Error::new(Kind::Config).with_message(format!(
                "profile {} has no [h3] table, so it has no HTTP/3 transport",
                profile.meta.name
            ))
        })?;
        let pseudo_order = match &h3.pseudo_order {
            Some(tokens) => parse_pseudo_order(tokens, "H3")?,
            None => DEFAULT_PSEUDO_ORDER,
        };
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
            max_response_body_bytes: crate::core::DEFAULT_MAX_BODY_SIZE as u64,
            tls: h3.tls.clone().unwrap_or_else(|| profile.tls.clone()),
            wire: h3.clone(),
            pseudo_order,
            max_header_list_bytes: h3
                .advertised_field_section_size()
                .unwrap_or(crate::core::DEFAULT_MAX_HEADER_LIST_BYTES as u64),
        })
    }
}

impl H3Profile {
    fn advertised_field_section_size(&self) -> Option<u64> {
        self.settings
            .iter()
            .flatten()
            .find(|setting| setting.id == Some(SETTINGS_MAX_FIELD_SECTION_SIZE))
            .and_then(|setting| setting.value)
            .or(self.max_field_section_size)
    }
}
