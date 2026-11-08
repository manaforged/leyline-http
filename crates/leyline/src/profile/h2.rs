use crate::error::Error;

/// The RFC 9113 initial flow-control window for a stream and for the connection.
pub(crate) const DEFAULT_WINDOW_SIZE: u32 = 65_535;
/// The Akamai PRIORITY field for a client that sends no PRIORITY frames.
pub(crate) const NO_PRIORITY_FRAMES: &str = "0";

/// An HTTP/2 SETTINGS parameter a profile can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum SettingId {
    /// SETTINGS_HEADER_TABLE_SIZE (0x1).
    HeaderTableSize = 1,
    /// SETTINGS_ENABLE_PUSH (0x2).
    EnablePush = 2,
    /// SETTINGS_MAX_CONCURRENT_STREAMS (0x3).
    MaxConcurrentStreams = 3,
    /// SETTINGS_INITIAL_WINDOW_SIZE (0x4).
    InitialWindowSize = 4,
    /// SETTINGS_MAX_FRAME_SIZE (0x5).
    MaxFrameSize = 5,
    /// SETTINGS_MAX_HEADER_LIST_SIZE (0x6).
    MaxHeaderListSize = 6,
    /// SETTINGS_ENABLE_CONNECT_PROTOCOL (0x8, RFC 8441).
    EnableConnectProtocol = 8,
    /// SETTINGS_NO_RFC7540_PRIORITIES (0x9, RFC 9218).
    NoRfc7540Priorities = 9,
}

struct SettingRow {
    id: SettingId,
    key: &'static str,
    field: &'static str,
}

const SETTINGS: [SettingRow; 8] = [
    SettingRow {
        id: SettingId::HeaderTableSize,
        key: "header_table_size",
        field: "header_table_size",
    },
    SettingRow {
        id: SettingId::EnablePush,
        key: "enable_push",
        field: "enable_push",
    },
    SettingRow {
        id: SettingId::MaxConcurrentStreams,
        key: "max_concurrent_streams",
        field: "max_concurrent_streams",
    },
    SettingRow {
        id: SettingId::InitialWindowSize,
        key: "initial_window_size",
        field: "initial_stream_window_size",
    },
    SettingRow {
        id: SettingId::MaxFrameSize,
        key: "max_frame_size",
        field: "max_frame_size",
    },
    SettingRow {
        id: SettingId::MaxHeaderListSize,
        key: "max_header_list_size",
        field: "max_header_list_size",
    },
    SettingRow {
        id: SettingId::EnableConnectProtocol,
        key: "unknown8",
        field: "unknown_setting8",
    },
    SettingRow {
        id: SettingId::NoRfc7540Priorities,
        key: "unknown9",
        field: "unknown_setting9",
    },
];

impl SettingId {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        SETTINGS.iter().map(|row| row.id)
    }

    /// Returns the setting a profile `settings_order` key names.
    pub fn parse_key(s: &str) -> Option<Self> {
        SETTINGS.iter().find(|row| row.key == s).map(|row| row.id)
    }

    pub(crate) fn from_code(code: u16) -> Option<Self> {
        SETTINGS
            .iter()
            .find(|row| row.id.code() == code)
            .map(|row| row.id)
    }

    pub(crate) fn from_field(name: &str) -> Option<Self> {
        SETTINGS
            .iter()
            .find(|row| row.field == name)
            .map(|row| row.id)
    }

    pub(crate) const fn code(self) -> u16 {
        self as u16
    }

    pub(crate) fn key(self) -> &'static str {
        SETTINGS
            .iter()
            .find(|row| row.id == self)
            .map_or("", |row| row.key)
    }

    pub(crate) fn field(self) -> &'static str {
        SETTINGS
            .iter()
            .find(|row| row.id == self)
            .map_or("", |row| row.field)
    }
}

/// The SETTINGS values a profile or a platform override sets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct H2Settings([Option<u32>; SETTINGS.len()]);

impl H2Settings {
    pub(crate) fn get(&self, id: SettingId) -> Option<u32> {
        SETTINGS
            .iter()
            .zip(self.0)
            .find(|(row, _)| row.id == id)
            .and_then(|(_, value)| value)
    }

    pub(crate) fn set(&mut self, id: SettingId, value: Option<u32>) -> Result<(), String> {
        if id == SettingId::EnablePush
            && let Some(other) = value.filter(|&v| v > 1)
        {
            return Err(format!("ENABLE_PUSH value {other} is not 0 or 1"));
        }
        if let Some((_, slot)) = SETTINGS
            .iter()
            .zip(self.0.iter_mut())
            .find(|(row, _)| row.id == id)
        {
            *slot = value;
        }
        Ok(())
    }
}

/// A request pseudo-header in the order a browser sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PseudoOrder {
    /// `:method`.
    Method,
    /// `:authority`.
    Authority,
    /// `:scheme`.
    Scheme,
    /// `:path`.
    Path,
}

const PSEUDO_KEYS: [(PseudoOrder, &str, &str); 4] = [
    (PseudoOrder::Method, "method", "m"),
    (PseudoOrder::Authority, "authority", "a"),
    (PseudoOrder::Scheme, "scheme", "s"),
    (PseudoOrder::Path, "path", "p"),
];

impl PseudoOrder {
    /// Returns the pseudo-header a profile `pseudo_order` key names.
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

    /// Returns the one-letter Akamai fingerprint label.
    pub fn label(&self) -> &'static str {
        PSEUDO_KEYS
            .iter()
            .find(|(order, _, _)| order == self)
            .map_or("", |&(_, _, label)| label)
    }
}

pub(crate) fn parse_pseudo_order(
    tokens: &[String],
    label: &str,
) -> Result<[PseudoOrder; 4], Error> {
    if tokens.len() != 4 {
        return Err(Error::config(format!(
            "{label} pseudo_order must have exactly 4 entries, got {}",
            tokens.len()
        )));
    }
    let mut pseudo_order = [
        PseudoOrder::Method,
        PseudoOrder::Authority,
        PseudoOrder::Scheme,
        PseudoOrder::Path,
    ];
    for (slot, s) in pseudo_order.iter_mut().zip(tokens.iter()) {
        *slot = PseudoOrder::parse_key(s)
            .ok_or_else(|| Error::config(format!("unknown {label} pseudo_order token: {s:?}")))?;
    }
    for i in 1..pseudo_order.len() {
        if pseudo_order[..i].contains(&pseudo_order[i]) {
            return Err(Error::config(format!(
                "duplicate {label} pseudo_order token: {:?}",
                tokens[i]
            )));
        }
    }
    Ok(pseudo_order)
}
