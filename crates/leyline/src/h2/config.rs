use std::time::Duration;

use crate::{Error, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum SettingId {
    HeaderTableSize = 1,
    EnablePush = 2,
    MaxConcurrentStreams = 3,
    InitialWindowSize = 4,
    MaxFrameSize = 5,
    MaxHeaderListSize = 6,
    Unknown8 = 8,
    Unknown9 = 9,
}

const SETTING_KEYS: [(SettingId, &str); 8] = [
    (SettingId::HeaderTableSize, "header_table_size"),
    (SettingId::EnablePush, "enable_push"),
    (SettingId::MaxConcurrentStreams, "max_concurrent_streams"),
    (SettingId::InitialWindowSize, "initial_window_size"),
    (SettingId::MaxFrameSize, "max_frame_size"),
    (SettingId::MaxHeaderListSize, "max_header_list_size"),
    (SettingId::Unknown8, "unknown8"),
    (SettingId::Unknown9, "unknown9"),
];

impl SettingId {
    pub fn parse_key(s: &str) -> Option<Self> {
        SETTING_KEYS
            .iter()
            .find(|(_, key)| *key == s)
            .map(|&(id, _)| id)
    }

    pub(crate) fn from_code(code: u16) -> Option<Self> {
        SETTING_KEYS
            .iter()
            .find(|&&(id, _)| id as u16 == code)
            .map(|&(id, _)| id)
    }

    pub(crate) fn key(self) -> &'static str {
        SETTING_KEYS
            .iter()
            .find(|&&(id, _)| id == self)
            .map_or("", |&(_, key)| key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PseudoOrder {
    Method,
    Authority,
    Scheme,
    Path,
}

const PSEUDO_KEYS: [(PseudoOrder, &str, &str); 4] = [
    (PseudoOrder::Method, "method", "m"),
    (PseudoOrder::Authority, "authority", "a"),
    (PseudoOrder::Scheme, "scheme", "s"),
    (PseudoOrder::Path, "path", "p"),
];

impl PseudoOrder {
    pub fn parse_key(s: &str) -> Option<Self> {
        PSEUDO_KEYS
            .iter()
            .find(|(_, key, _)| *key == s)
            .map(|&(order, _, _)| order)
    }

    pub(crate) fn key_for_label(label: &str) -> Option<&'static str> {
        PSEUDO_KEYS
            .iter()
            .find(|(_, _, known)| *known == label)
            .map(|&(_, key, _)| key)
    }

    pub fn label(&self) -> &'static str {
        PSEUDO_KEYS
            .iter()
            .find(|(order, _, _)| order == self)
            .map_or("", |&(_, _, label)| label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityParams {
    pub exclusive: bool,
    pub stream_dependency: u32,
    pub weight: u8,
}

#[derive(Debug, Clone)]
pub struct H2Config {
    pub settings: Vec<(SettingId, u32)>,
    pub settings_order: Vec<SettingId>,
    pub pseudo_order: [PseudoOrder; 4],
    pub initial_connection_window_size: u32,
    pub default_priority: Option<PriorityParams>,
    pub rst_stream_flood_threshold: u32,
    pub rst_stream_flood_window: Duration,
    pub max_response_body_bytes: usize,
    pub max_header_block_bytes: usize,
    pub settings_flood_threshold: u32,
    pub settings_flood_window: Duration,
    pub header_block_reassembly_timeout: Duration,
}

impl H2Config {
    pub(crate) fn advertised_initial_window_size(&self) -> u32 {
        self.settings
            .iter()
            .find(|(id, _)| *id == SettingId::InitialWindowSize)
            .map(|(_, v)| *v)
            .unwrap_or(65_535)
    }

    pub fn from_profile(h2: &crate::profile::H2Profile) -> Result<Self, Error> {
        use std::collections::HashMap;

        let mut available: HashMap<SettingId, u32> = HashMap::new();
        if let Some(v) = h2.header_table_size {
            available.insert(SettingId::HeaderTableSize, v);
        }
        if let Some(v) = h2.enable_push {
            available.insert(SettingId::EnablePush, if v { 1 } else { 0 });
        }
        if let Some(v) = h2.max_concurrent_streams {
            available.insert(SettingId::MaxConcurrentStreams, v);
        }
        if let Some(v) = h2.initial_stream_window_size {
            available.insert(SettingId::InitialWindowSize, v);
        }
        if let Some(v) = h2.max_frame_size {
            available.insert(SettingId::MaxFrameSize, v);
        }
        if let Some(v) = h2.max_header_list_size {
            available.insert(SettingId::MaxHeaderListSize, v);
        }
        if let Some(v) = h2.unknown_setting8 {
            available.insert(SettingId::Unknown8, v);
        }
        if let Some(v) = h2.unknown_setting9 {
            available.insert(SettingId::Unknown9, v);
        }

        let settings_order: Vec<SettingId> = h2
            .settings_order
            .iter()
            .map(|s| {
                SettingId::parse_key(s).ok_or_else(|| {
                    Error::new(Kind::Config)
                        .with_message(format!("unknown H2 settings_order key: {s:?}"))
                })
            })
            .collect::<Result<_, _>>()?;

        let settings: Vec<(SettingId, u32)> = settings_order
            .iter()
            .filter_map(|id| available.get(id).map(|v| (*id, *v)))
            .collect();

        if h2.pseudo_order.len() != 4 {
            return Err(Error::new(Kind::Config).with_message(format!(
                "H2 pseudo_order must have exactly 4 entries, got {}",
                h2.pseudo_order.len()
            )));
        }
        let mut pseudo_order = [
            PseudoOrder::Method,
            PseudoOrder::Authority,
            PseudoOrder::Scheme,
            PseudoOrder::Path,
        ];
        for (slot, s) in pseudo_order.iter_mut().zip(h2.pseudo_order.iter()) {
            *slot = PseudoOrder::parse_key(s).ok_or_else(|| {
                Error::new(Kind::Config)
                    .with_message(format!("unknown H2 pseudo_order token: {s:?}"))
            })?;
        }
        for i in 1..pseudo_order.len() {
            if pseudo_order[..i].contains(&pseudo_order[i]) {
                return Err(Error::new(Kind::Config).with_message(format!(
                    "duplicate H2 pseudo_order token: {:?}",
                    h2.pseudo_order[i]
                )));
            }
        }

        let initial_connection_window_size =
            h2.initial_connection_window_size.ok_or_else(|| {
                Error::new(Kind::Config)
                    .with_message("H2 profile missing initial_connection_window_size".to_string())
            })?;

        Ok(Self {
            settings,
            settings_order,
            pseudo_order,
            initial_connection_window_size,
            default_priority: h2.default_priority.map(|p| PriorityParams {
                exclusive: p.exclusive,
                stream_dependency: p.stream_dependency,
                weight: p.weight,
            }),
            rst_stream_flood_threshold: 100,
            rst_stream_flood_window: Duration::from_secs(10),
            max_response_body_bytes: crate::core::DEFAULT_MAX_BODY_SIZE,
            max_header_block_bytes: 256 * 1024,
            settings_flood_threshold: 20,
            settings_flood_window: Duration::from_secs(10),
            header_block_reassembly_timeout: Duration::from_secs(10),
        })
    }

    pub fn akamai_fingerprint(&self) -> String {
        let settings_str: String = self
            .settings_order
            .iter()
            .filter_map(|id| {
                self.settings
                    .iter()
                    .find(|(sid, _)| sid == id)
                    .map(|(sid, val)| format!("{}:{}", *sid as u16, val))
            })
            .collect::<Vec<_>>()
            .join(";");

        let window_update = self.initial_connection_window_size.saturating_sub(65535);

        let pseudo_str: String = self
            .pseudo_order
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(",");

        format!("{}|{}|0|{}", settings_str, window_update, pseudo_str)
    }
}

#[cfg(test)]
mod tests;
