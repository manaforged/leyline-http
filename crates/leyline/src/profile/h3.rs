use std::collections::HashSet;

use serde::Deserialize;

use crate::profile::TlsProfile;

const LOCAL_TRANSPORT_PARAMS: [u64; 14] = [
    0x01, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0e, 0x0f, 0x20,
];
const ENFORCED_TRANSPORT_PARAMS: [u64; 8] = [0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0e, 0x0f];
const SERVER_TRANSPORT_PARAMS: [u64; 4] = [0x00, 0x02, 0x0d, 0x10];
const INITIAL_SOURCE_CONNECTION_ID: u64 = 0x0f;
const HTTP2_RESERVED_SETTINGS: [u64; 4] = [0x02, 0x03, 0x04, 0x05];
const MAX_VARINT: u64 = (1 << 62) - 1;

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H3Profile {
    pub initial_max_data: u64,
    pub initial_max_stream_data_bidi_local: u64,
    pub initial_max_stream_data_bidi_remote: u64,
    pub initial_max_stream_data_uni: u64,
    pub initial_max_streams_bidi: u64,
    pub initial_max_streams_uni: u64,
    pub max_idle_timeout_secs: u64,
    pub max_udp_payload_size: u16,
    pub active_connection_id_limit: u64,
    pub dcid_length: H3ConnectionIdLength,
    #[serde(default)]
    pub scid_length: Option<usize>,
    #[serde(default)]
    pub qpack_max_table_capacity: Option<u64>,
    #[serde(default)]
    pub qpack_blocked_streams: Option<u64>,
    #[serde(default)]
    pub max_field_section_size: Option<u64>,
    #[serde(default)]
    pub race: bool,
    #[serde(default)]
    pub max_ack_delay_ms: Option<u64>,
    #[serde(default)]
    pub transport_parameters: Option<Vec<H3TransportParam>>,
    #[serde(default)]
    pub transport_order: H3Order,
    #[serde(default)]
    pub settings: Option<Vec<H3Setting>>,
    #[serde(default)]
    pub control_grease_frame: Option<H3Grease>,
    #[serde(default)]
    pub pseudo_order: Option<Vec<String>>,
    #[serde(default)]
    pub priority_update: bool,
    #[serde(default)]
    pub tls: Option<TlsProfile>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum H3ConnectionIdLength {
    Fixed(usize),
    Weighted { weights: Vec<(usize, u32)> },
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum H3Order {
    #[default]
    Fixed,
    Shuffle,
    Rotate,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H3Grease {
    pub id_bits: u32,
    #[serde(default)]
    pub max_len: usize,
    #[serde(default)]
    pub value_bits: u32,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H3TransportParam {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub varint: Option<u64>,
    #[serde(default)]
    pub hex: Option<String>,
    #[serde(default)]
    pub grease: Option<H3Grease>,
    #[serde(default)]
    pub versions: Option<H3VersionInformation>,
    #[serde(default)]
    pub pinned: bool,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H3VersionInformation {
    pub chosen: u32,
    pub available: Vec<u32>,
    #[serde(default)]
    pub grease: Option<H3VersionGrease>,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum H3VersionGrease {
    First,
    Random,
}

#[expect(
    missing_docs,
    reason = "profile schema mirrors the embedded TOML tables; variant and field names are the documentation"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct H3Setting {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub value: Option<u64>,
    #[serde(default)]
    pub grease: Option<H3Grease>,
}

impl H3Profile {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let legacy_settings = self.qpack_max_table_capacity.is_some()
            || self.qpack_blocked_streams.is_some()
            || self.max_field_section_size.is_some();
        if self.settings.is_some() == legacy_settings {
            return Err(
                "[h3] needs either a settings list or the qpack_max_table_capacity, \
                 qpack_blocked_streams and max_field_section_size fields, not both"
                    .into(),
            );
        }
        if let Some(settings) = &self.settings {
            validate_settings(settings)?;
        }
        if let Some(tls) = &self.tls {
            crate::profile::permutation::validate(tls, true)?;
        }
        if let Some(params) = &self.transport_parameters {
            validate_transport_parameters(params)?;
        }
        if let H3ConnectionIdLength::Weighted { weights } = &self.dcid_length
            && weights.iter().all(|(_, weight)| *weight == 0)
        {
            return Err("[h3] dcid_length weights must not all be zero".into());
        }
        Ok(())
    }
}

fn validate_settings(settings: &[H3Setting]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for setting in settings {
        let (id, value) = match (setting.id, setting.value, &setting.grease) {
            (None, None, Some(_)) => continue,
            (Some(id), Some(value), None) => (id, value),
            _ => return Err("[h3] settings entry needs either id and value, or grease".into()),
        };
        if id > MAX_VARINT || value > MAX_VARINT {
            return Err(format!("[h3] setting {id} does not fit in a QUIC varint"));
        }
        if HTTP2_RESERVED_SETTINGS.contains(&id) {
            return Err(format!("[h3] setting {id} is reserved for HTTP/2"));
        }
        if !seen.insert(id) {
            return Err(format!("[h3] setting {id} appears twice"));
        }
    }
    Ok(())
}

fn validate_transport_parameters(params: &[H3TransportParam]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for param in params {
        param.validate()?;
        if let Some(id) = param.id
            && !seen.insert(id)
        {
            return Err(format!("[h3] transport parameter {id} appears twice"));
        }
    }
    if !seen.contains(&INITIAL_SOURCE_CONNECTION_ID) {
        return Err(format!(
            "[h3] transport_parameters must list initial_source_connection_id \
             ({INITIAL_SOURCE_CONNECTION_ID})"
        ));
    }
    Ok(())
}

impl H3TransportParam {
    fn validate(&self) -> Result<(), String> {
        let kinds = [
            self.varint.is_some(),
            self.hex.is_some(),
            self.grease.is_some(),
            self.versions.is_some(),
        ]
        .iter()
        .filter(|set| **set)
        .count();
        match (self.id, kinds) {
            (Some(id), _) if id > MAX_VARINT => Err(format!(
                "[h3] transport parameter {id} does not fit in a QUIC varint"
            )),
            (Some(id), _) if SERVER_TRANSPORT_PARAMS.contains(&id) => Err(format!(
                "[h3] transport parameter {id} is sent only by servers"
            )),
            (Some(id), 0) if LOCAL_TRANSPORT_PARAMS.contains(&id) => Ok(()),
            (Some(id), 0) => Err(format!(
                "[h3] transport parameter {id} has no value and is not one Leyline fills in"
            )),
            (Some(id), 1) if ENFORCED_TRANSPORT_PARAMS.contains(&id) => Err(format!(
                "[h3] transport parameter {id} is enforced locally, so it takes no raw value"
            )),
            (Some(id), 1) if self.varint.is_some_and(|value| value > MAX_VARINT) => Err(format!(
                "[h3] transport parameter {id} value does not fit in a QUIC varint"
            )),
            (Some(_), 1) if self.versions.is_some() => self
                .versions
                .as_ref()
                .map_or(Ok(()), H3VersionInformation::validate),
            (Some(_), 1) if self.grease.is_none() => Ok(()),
            (None, 1) if self.grease.is_some() => Ok(()),
            _ => Err(format!(
                "[h3] transport parameter entry {self:?} needs an id with at most one of varint, \
                 hex or versions, or a grease table alone"
            )),
        }
    }
}

impl H3VersionInformation {
    fn validate(&self) -> Result<(), String> {
        match std::iter::once(&self.chosen)
            .chain(&self.available)
            .find(|version| !quic_version_supported(**version))
        {
            Some(version) => Err(format!(
                "[h3] QUIC version {version:#010x} is not supported by the QUIC stack"
            )),
            None => Ok(()),
        }
    }
}

#[cfg(feature = "http3")]
fn quic_version_supported(version: u32) -> bool {
    leyline_quiche::version_is_supported(version)
}

#[cfg(not(feature = "http3"))]
fn quic_version_supported(_version: u32) -> bool {
    true
}
