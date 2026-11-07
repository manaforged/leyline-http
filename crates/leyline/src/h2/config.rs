use std::time::Duration;

use crate::error::Error;
use crate::h2::frame::{DEFAULT_HEADER_TABLE_SIZE, DEFAULT_MAX_FRAME_SIZE, StreamDependency};
use crate::profile::{DEFAULT_WINDOW_SIZE, PseudoOrder, SettingId, parse_pseudo_order};

#[derive(Debug, Clone)]
pub struct H2Config {
    pub settings: Vec<(SettingId, u32)>,
    pub pseudo_order: [PseudoOrder; 4],
    pub initial_connection_window_size: u32,
    pub default_priority: Option<StreamDependency>,
    /// Abuse limits that only benches and tests change.
    pub limits: H2Limits,
}

/// Abuse limits for one HTTP/2 connection.
#[derive(Debug, Clone, Copy)]
pub struct H2Limits {
    /// RST_STREAM frames tolerated per window.
    pub rst_stream_flood_threshold: u32,
    /// The window that counts RST_STREAM frames.
    pub rst_stream_flood_window: Duration,
    /// SETTINGS frames tolerated per window.
    pub settings_flood_threshold: u32,
    /// The window that counts SETTINGS frames.
    pub settings_flood_window: Duration,
    /// PING, WINDOW_UPDATE on unknown streams, and empty CONTINUATION frames tolerated per window.
    pub control_flood_threshold: u32,
    /// The window that counts control frames.
    pub control_flood_window: Duration,
    /// The longest wait for the CONTINUATION frames of one header block.
    pub header_block_reassembly_timeout: Duration,
}

const LIMITS: H2Limits = H2Limits {
    rst_stream_flood_threshold: 100,
    rst_stream_flood_window: Duration::from_secs(10),
    settings_flood_threshold: 20,
    settings_flood_window: Duration::from_secs(10),
    control_flood_threshold: 1000,
    control_flood_window: Duration::from_secs(10),
    header_block_reassembly_timeout: Duration::from_secs(10),
};

impl Default for H2Limits {
    fn default() -> Self {
        LIMITS
    }
}

const DEFAULT_SETTINGS: [(SettingId, u32); 4] = [
    (SettingId::HeaderTableSize, DEFAULT_HEADER_TABLE_SIZE),
    (SettingId::EnablePush, 0),
    (SettingId::InitialWindowSize, DEFAULT_WINDOW_SIZE),
    (SettingId::MaxFrameSize, DEFAULT_MAX_FRAME_SIZE),
];

impl Default for H2Config {
    fn default() -> Self {
        Self {
            settings: DEFAULT_SETTINGS.to_vec(),
            pseudo_order: [
                PseudoOrder::Method,
                PseudoOrder::Authority,
                PseudoOrder::Scheme,
                PseudoOrder::Path,
            ],
            initial_connection_window_size: DEFAULT_WINDOW_SIZE,
            default_priority: None,
            limits: LIMITS,
        }
    }
}

impl H2Config {
    pub(crate) fn setting(&self, id: SettingId) -> Option<u32> {
        self.settings
            .iter()
            .find(|(sid, _)| *sid == id)
            .map(|(_, v)| *v)
    }

    pub(crate) fn limits(&self) -> &H2Limits {
        &self.limits
    }

    pub(crate) fn header_list_limit(&self) -> usize {
        self.setting(SettingId::MaxHeaderListSize)
            .map_or(crate::wire::DEFAULT_MAX_HEADER_LIST_BYTES, |v| v as usize)
    }

    pub(crate) fn advertised_initial_window_size(&self) -> u32 {
        self.setting(SettingId::InitialWindowSize)
            .unwrap_or(DEFAULT_WINDOW_SIZE)
    }

    pub fn from_profile(h2: &crate::profile::H2Profile) -> Result<Self, Error> {
        let settings_order: Vec<SettingId> = h2
            .settings_order
            .iter()
            .map(|s| {
                SettingId::parse_key(s)
                    .ok_or_else(|| Error::config(format!("unknown H2 settings_order key: {s:?}")))
            })
            .collect::<Result<_, _>>()?;

        let settings: Vec<(SettingId, u32)> = settings_order
            .into_iter()
            .filter_map(|id| h2.settings.get(id).map(|v| (id, v)))
            .collect();

        let pseudo_order = parse_pseudo_order(&h2.pseudo_order, "H2")?;

        let initial_connection_window_size =
            h2.initial_connection_window_size.ok_or_else(|| {
                Error::config("H2 profile missing initial_connection_window_size".to_string())
            })?;

        Ok(Self {
            settings,
            pseudo_order,
            initial_connection_window_size,
            default_priority: h2.default_priority.map(|p| StreamDependency {
                exclusive: p.exclusive,
                dependency_id: p.stream_dependency,
                weight: p.weight,
            }),
            limits: LIMITS,
        })
    }
}

#[cfg(test)]
mod tests;
