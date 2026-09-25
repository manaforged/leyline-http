use serde::Deserialize;

use crate::profile::TlsProfile;

const LOCAL_TRANSPORT_PARAMS: [u64; 14] = [
    0x01, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0e, 0x0f, 0x20,
];

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
        for setting in self.settings.iter().flatten() {
            if setting.grease.is_some() == (setting.id.is_some() || setting.value.is_some()) {
                return Err("[h3] settings entry needs either id and value, or grease".into());
            }
        }
        if let Some(tls) = &self.tls {
            crate::profile::permutation::validate(tls, true)?;
        }
        for param in self.transport_parameters.iter().flatten() {
            param.validate()?;
        }
        if let H3ConnectionIdLength::Weighted { weights } = &self.dcid_length
            && weights.iter().all(|(_, weight)| *weight == 0)
        {
            return Err("[h3] dcid_length weights must not all be zero".into());
        }
        Ok(())
    }
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
            (Some(id), 0) if LOCAL_TRANSPORT_PARAMS.contains(&id) => Ok(()),
            (Some(id), 0) => Err(format!(
                "[h3] transport parameter {id} has no value and is not one Leyline fills in"
            )),
            (Some(_), 1) if self.grease.is_none() => Ok(()),
            (None, 1) if self.grease.is_some() => Ok(()),
            _ => Err(format!(
                "[h3] transport parameter entry {self:?} needs an id with at most one of varint, \
                 hex or versions, or a grease table alone"
            )),
        }
    }
}
