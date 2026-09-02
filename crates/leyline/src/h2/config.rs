//! HTTP/2 SETTINGS frame and pseudo-header ordering.

use std::time::Duration;

use crate::{Error, Kind};

/// HTTP/2 SETTINGS parameter ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum SettingId {
    /// SETTINGS_HEADER_TABLE_SIZE (0x1)
    HeaderTableSize = 1,
    /// SETTINGS_ENABLE_PUSH (0x2)
    EnablePush = 2,
    /// SETTINGS_MAX_CONCURRENT_STREAMS (0x3)
    MaxConcurrentStreams = 3,
    /// SETTINGS_INITIAL_WINDOW_SIZE (0x4)
    InitialWindowSize = 4,
    /// SETTINGS_MAX_FRAME_SIZE (0x5)
    MaxFrameSize = 5,
    /// SETTINGS_MAX_HEADER_LIST_SIZE (0x6)
    MaxHeaderListSize = 6,
    /// Unknown setting 8 (EnableConnectProtocol in some implementations).
    Unknown8 = 8,
    /// Unknown setting 9.
    Unknown9 = 9,
}

/// RFC 8441 §3 — `SETTINGS_ENABLE_CONNECT_PROTOCOL` identifier (0x8).
pub const SETTINGS_ENABLE_CONNECT_PROTOCOL: u16 = 0x8;

impl SettingId {
    /// Parse from the key string used in TOML profiles.
    pub fn parse_key(s: &str) -> Option<Self> {
        match s {
            "header_table_size" => Some(Self::HeaderTableSize),
            "enable_push" => Some(Self::EnablePush),
            "max_concurrent_streams" => Some(Self::MaxConcurrentStreams),
            "initial_window_size" => Some(Self::InitialWindowSize),
            "max_frame_size" => Some(Self::MaxFrameSize),
            "max_header_list_size" => Some(Self::MaxHeaderListSize),
            "unknown8" => Some(Self::Unknown8),
            "unknown9" => Some(Self::Unknown9),
            _ => None,
        }
    }
}

/// HTTP/2 pseudo-header ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PseudoOrder {
    /// `:method`
    Method,
    /// `:authority`
    Authority,
    /// `:scheme`
    Scheme,
    /// `:path`
    Path,
}

impl PseudoOrder {
    /// Parse from the key string used in TOML profiles.
    pub fn parse_key(s: &str) -> Option<Self> {
        match s {
            "method" => Some(Self::Method),
            "authority" => Some(Self::Authority),
            "scheme" => Some(Self::Scheme),
            "path" => Some(Self::Path),
            _ => None,
        }
    }

    /// Short label for fingerprint display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Method => "m",
            Self::Authority => "a",
            Self::Scheme => "s",
            Self::Path => "p",
        }
    }
}

/// Priority fields emitted alongside a client-initiated HEADERS frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityParams {
    /// Exclusive dependency bit (E) — when set, the new stream becomes the sole dependency of `stream_dependency`.
    pub exclusive: bool,
    /// Stream ID this new stream depends on (0 = root of the tree).
    pub stream_dependency: u32,
    /// Weight on the wire (`actual_weight - 1`); range `0..=255`.
    pub weight: u8,
}

/// Resolved HTTP/2 fingerprint configuration.
#[derive(Debug, Clone)]
pub struct H2Config {
    /// Ordered SETTINGS parameters with their values.
    pub settings: Vec<(SettingId, u32)>,
    /// SETTINGS frame parameter ordering.
    pub settings_order: Vec<SettingId>,
    /// Pseudo-header ordering for HEADERS frames.
    pub pseudo_order: [PseudoOrder; 4],
    /// Initial connection-level window size (for WINDOW_UPDATE after preface).
    pub initial_connection_window_size: u32,
    /// Optional PRIORITY fields to emit on the initial HEADERS frame of each request.
    pub default_priority: Option<PriorityParams>,
    /// Threshold for the inbound RST_STREAM flood guard — more than this many RST_STREAM frames inside `rst_stream_flood_window` causes the connection to tear down with `ENHANCE_YOUR_CALM` (CVE-2023-44487 defense-in-depth).
    pub rst_stream_flood_threshold: u32,
    /// Sliding window over which `rst_stream_flood_threshold` is measured.
    pub rst_stream_flood_window: Duration,
    /// Hard cap on the size of a response body.
    pub max_response_body_bytes: usize,
    /// Hard cap on the total size of a single inbound header block (HEADERS + all subsequent CONTINUATION fragments).
    pub max_header_block_bytes: usize,
    /// Threshold for the inbound non-ACK SETTINGS flood guard — more than this many mid-connection SETTINGS updates inside `settings_flood_window` causes the connection to tear down with `ENHANCE_YOUR_CALM`.
    pub settings_flood_threshold: u32,
    /// Sliding window over which `settings_flood_threshold` is measured.
    pub settings_flood_window: Duration,
    /// Wall-clock ceiling on reassembling a single inbound header block (HEADERS + all subsequent CONTINUATION frames).
    pub header_block_reassembly_timeout: Duration,
}

impl H2Config {
    /// The per-stream receive window we advertise to the peer via `SETTINGS_INITIAL_WINDOW_SIZE`, falling back to the RFC 9113 §6.5.2 default when the profile omits the setting.
    pub(crate) fn advertised_initial_window_size(&self) -> u32 {
        self.settings
            .iter()
            .find(|(id, _)| *id == SettingId::InitialWindowSize)
            .map(|(_, v)| *v)
            .unwrap_or(65_535)
    }

    /// Build from a TOML H2Profile.
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
            max_response_body_bytes: 100 * 1024 * 1024,
            max_header_block_bytes: 256 * 1024,
            settings_flood_threshold: 20,
            settings_flood_window: Duration::from_secs(10),
            header_block_reassembly_timeout: Duration::from_secs(10),
        })
    }

    /// Compute the Akamai-style H2 fingerprint string.
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
