//! HTTP/2 SETTINGS frame and pseudo-header ordering.
//!
//! Defines the configuration types for HTTP/2 fingerprinting. These are
//! resolved from TOML browser profiles and applied to hyper2 connections.

/// HTTP/2 SETTINGS parameter ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
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
}

impl H2Config {
    /// Build from a TOML H2Profile.
    ///
    /// Settings are stored in the order specified by `settings_order` so the
    /// wire format matches the profile's declared ordering.
    pub fn from_profile(h2: &leyline_profile::H2Profile) -> Self {
        use std::collections::HashMap;

        // Collect all available settings into a lookup map.
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

        // Build settings in the declared order.
        let settings_order: Vec<SettingId> = h2
            .settings_order
            .iter()
            .filter_map(|s| SettingId::parse_key(s))
            .collect();

        let settings: Vec<(SettingId, u32)> = settings_order
            .iter()
            .filter_map(|id| available.get(id).map(|v| (*id, *v)))
            .collect();

        let mut pseudo_order = [
            PseudoOrder::Method,
            PseudoOrder::Authority,
            PseudoOrder::Scheme,
            PseudoOrder::Path,
        ];
        for (i, s) in h2.pseudo_order.iter().enumerate().take(4) {
            if let Some(p) = PseudoOrder::parse_key(s) {
                pseudo_order[i] = p;
            }
        }

        let initial_connection_window_size = h2.initial_connection_window_size.unwrap_or(65535);

        Self {
            settings,
            settings_order,
            pseudo_order,
            initial_connection_window_size,
        }
    }

    /// Compute the Akamai-style H2 fingerprint string.
    pub fn akamai_fingerprint(&self) -> String {
        // SETTINGS part: ordered by settings_order
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

        // WINDOW_UPDATE: connection window - default 65535
        let window_update = self.initial_connection_window_size.saturating_sub(65535);

        // Pseudo-header order
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
mod tests {
    use super::*;

    #[test]
    fn chrome147_h2_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        let profile = reg.get("chrome", 147).unwrap();
        let h2 = H2Config::from_profile(&profile.h2);
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144;8:1|15663105|0|m,a,s,p");
    }

    #[test]
    fn firefox148_h2_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        let profile = reg.get("firefox", 148).unwrap();
        let h2 = H2Config::from_profile(&profile.h2);
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "1:65536;2:0;4:131072;5:16384|12517377|0|m,p,a,s");
    }

    #[test]
    fn okhttp_h2_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        let profile = reg.get("okhttp", 10).unwrap();
        let h2 = H2Config::from_profile(&profile.h2);
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "4:16777216|16711681|0|m,p,a,s");
    }

    #[test]
    fn safari18_h2_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        let profile = reg.get("safari", 18).unwrap();
        let h2 = H2Config::from_profile(&profile.h2);
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "2:0;3:100;4:2097152;8:1;9:1|10420225|0|m,s,a,p");
    }

    #[test]
    fn safari_ios18_h2_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        let profile = reg.get("safari-ios", 18).unwrap();
        let h2 = H2Config::from_profile(&profile.h2);
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "2:0;3:100;4:2097152;9:1|10420225|0|m,s,a,p");
    }

    #[test]
    fn all_profiles_produce_expected_fingerprint() {
        let reg = leyline_profile::ProfileRegistry::builtin();
        for browser in [
            ("chrome", 145),
            ("chrome", 146),
            ("chrome", 147),
            ("firefox", 148),
            ("safari", 18),
            ("safari-ios", 15),
            ("safari-ios", 17),
            ("safari-ios", 18),
            ("okhttp", 10),
            ("okhttp", 7),
        ] {
            let profile = reg
                .get(browser.0, browser.1)
                .unwrap_or_else(|| panic!("missing profile: {} {}", browser.0, browser.1));
            if let Some(expected) = profile.expected_h2_fingerprint() {
                let h2 = H2Config::from_profile(&profile.h2);
                let actual = h2.akamai_fingerprint();
                assert_eq!(
                    actual, expected,
                    "H2 fingerprint mismatch for {} {}",
                    browser.0, browser.1
                );
            }
        }
    }
}
