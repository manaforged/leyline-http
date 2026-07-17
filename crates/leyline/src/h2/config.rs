//! HTTP/2 SETTINGS frame and pseudo-header ordering.
//!
//! Defines the configuration types for HTTP/2 fingerprinting. These are
//! resolved from TOML browser profiles and applied to hyper2 connections.

use std::time::Duration;

/// HTTP/2 SETTINGS parameter ID.
///
/// `#[non_exhaustive]` so a new RFC parameter can be added without
/// breaking downstream `match` arms.
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
///
/// Value `1` on the server side means "I accept extended CONNECT with a
/// `:protocol` pseudo-header" (e.g. WebSocket over HTTP/2); value `0`
/// (or the setting missing entirely) means the server only speaks the
/// classic CONNECT tunnel shape from RFC 9113 §8.5. Per RFC 8441 §3
/// the setting is sticky — once the server advertises `1` it cannot
/// revert to `0` on the same connection.
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
///
/// RFC 9113 deprecated the PRIORITY flag, but Chrome and Firefox still emit
/// stream-dependency data on initial HEADERS for fingerprint parity. Weight
/// is carried on the wire as `weight - 1`, so the range `0..=255` maps to
/// priorities `1..=256`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityParams {
    /// Exclusive dependency bit (E) — when set, the new stream becomes the
    /// sole dependency of `stream_dependency`.
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
    /// Optional PRIORITY fields to emit on the initial HEADERS frame of
    /// each request. `None` (the default) sends no priority data — matches
    /// non-browser HTTP/2 clients. Set to `Some(..)` to match Chrome /
    /// Firefox fingerprints.
    pub default_priority: Option<PriorityParams>,
    /// Threshold for the inbound RST_STREAM flood guard — more than this
    /// many RST_STREAM frames inside `rst_stream_flood_window` causes the
    /// connection to tear down with `ENHANCE_YOUR_CALM` (CVE-2023-44487
    /// defense-in-depth).
    pub rst_stream_flood_threshold: u32,
    /// Sliding window over which `rst_stream_flood_threshold` is measured.
    pub rst_stream_flood_window: Duration,
    /// Max time to wait for the peer to ACK our SETTINGS frame during
    /// handshake (RFC 9113 §6.5.3). On timeout, the handshake fails
    /// with a `SettingsTimeout` connection error. Default: 10 s.
    pub settings_ack_timeout: Duration,
    /// Hard cap on the size of a response body. Requests that exceed
    /// this value fail with a stream-level error; existing streams on
    /// the same connection continue. Default: 100 MiB.
    pub max_response_body_bytes: usize,
    /// Hard cap on the total size of a single inbound header block
    /// (HEADERS + all subsequent CONTINUATION fragments). Exceeding it
    /// is a `CompressionError`. Default: 256 KiB (Chrome's ceiling).
    pub max_header_block_bytes: usize,
    /// Threshold for the inbound non-ACK SETTINGS flood guard — more
    /// than this many mid-connection SETTINGS updates inside
    /// `settings_flood_window` causes the connection to tear down
    /// with `ENHANCE_YOUR_CALM`. Defence against a peer that burns
    /// client CPU by bursting SETTINGS frames (each triggers an ACK
    /// write plus a stream-window rescan). Default: 20.
    pub settings_flood_threshold: u32,
    /// Sliding window over which `settings_flood_threshold` is
    /// measured. Default: 10 s.
    pub settings_flood_window: Duration,
    /// Wall-clock ceiling on reassembling a single inbound header block
    /// (HEADERS + all subsequent CONTINUATION frames). Reassembly blocks
    /// the single driver task, so a peer that sends HEADERS without
    /// END_HEADERS and then withholds (or dribbles) CONTINUATION bytes
    /// would otherwise stall every multiplexed stream on the connection
    /// indefinitely — `max_header_block_bytes` bounds size, never time.
    /// Exceeding it is a `ProtocolError`. Default: 10 s.
    pub header_block_reassembly_timeout: Duration,
}

impl H2Config {
    /// The per-stream receive window we advertise to the peer via
    /// `SETTINGS_INITIAL_WINDOW_SIZE`, falling back to the RFC 9113
    /// §6.5.2 default when the profile omits the setting. This is the
    /// only correct basis for seeding and replenishing a stream's
    /// receive window — `initial_connection_window_size` is the
    /// connection-level value and `peer_settings.initial_window_size`
    /// governs the send direction.
    pub(crate) fn advertised_initial_window_size(&self) -> u32 {
        self.settings
            .iter()
            .find(|(id, _)| *id == SettingId::InitialWindowSize)
            .map(|(_, v)| *v)
            .unwrap_or(65_535)
    }

    /// Build from a TOML H2Profile.
    ///
    /// Settings are stored in the order specified by `settings_order` so the
    /// wire format matches the profile's declared ordering.
    pub fn from_profile(h2: &crate::profile::H2Profile) -> Result<Self, crate::Error> {
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

        // Reject typo'd keys; a silently dropped key ships the wrong SETTINGS frame.
        let settings_order: Vec<SettingId> = h2
            .settings_order
            .iter()
            .map(|s| {
                SettingId::parse_key(s).ok_or_else(|| {
                    crate::Error::Config(format!("unknown H2 settings_order key: {s:?}"))
                })
            })
            .collect::<Result<_, _>>()?;

        // A valueless key is an ordering-only entry (e.g. Chrome 147 lists
        // max_frame_size / max_concurrent_streams with no value); it is omitted.
        let settings: Vec<(SettingId, u32)> = settings_order
            .iter()
            .filter_map(|id| available.get(id).map(|v| (*id, *v)))
            .collect();

        // pseudo_order must be exactly 4 known tokens; a short/typo'd list would
        // silently keep a Chrome default slot, mis-fingerprinting Firefox/Safari.
        if h2.pseudo_order.len() != 4 {
            return Err(crate::Error::Config(format!(
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
                crate::Error::Config(format!("unknown H2 pseudo_order token: {s:?}"))
            })?;
        }
        // Four *known* tokens is not enough — a duplicate means another
        // pseudo-header is missing, and `build_pseudo_list` would emit
        // one twice and drop the other (a malformed request, not just a
        // wrong fingerprint).
        for i in 1..pseudo_order.len() {
            if pseudo_order[..i].contains(&pseudo_order[i]) {
                return Err(crate::Error::Config(format!(
                    "duplicate H2 pseudo_order token: {:?}",
                    h2.pseudo_order[i]
                )));
            }
        }

        // Missing window → no WINDOW_UPDATE (increment 0) → Akamai shows |0|.
        let initial_connection_window_size =
            h2.initial_connection_window_size.ok_or_else(|| {
                crate::Error::Config(
                    "H2 profile missing initial_connection_window_size".to_string(),
                )
            })?;

        Ok(Self {
            settings,
            settings_order,
            pseudo_order,
            initial_connection_window_size,
            default_priority: None,
            rst_stream_flood_threshold: 100,
            rst_stream_flood_window: Duration::from_secs(10),
            settings_ack_timeout: Duration::from_secs(10),
            max_response_body_bytes: 100 * 1024 * 1024,
            max_header_block_bytes: 256 * 1024,
            settings_flood_threshold: 20,
            settings_flood_window: Duration::from_secs(10),
            header_block_reassembly_timeout: Duration::from_secs(10),
        })
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
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("chrome", 147).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
    }

    #[test]
    fn chrome148_h2_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("chrome", 148).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p");
    }

    #[test]
    fn firefox150_h2_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("firefox", 150).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "1:65536;2:0;4:131072;5:16384|12517377|0|m,p,a,s");
    }

    #[test]
    fn okhttp_h2_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("okhttp", 10).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "4:16777216|16711681|0|m,p,a,s");
    }

    #[test]
    fn safari18_h2_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("safari", 18).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "2:0;3:100;4:2097152;8:1;9:1|10420225|0|m,s,a,p");
    }

    #[test]
    fn safari_ios18_h2_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        let profile = reg.get("safari-ios", 18).unwrap();
        let h2 = H2Config::from_profile(&profile.h2).unwrap();
        let fp = h2.akamai_fingerprint();
        assert_eq!(fp, "2:0;3:100;4:2097152;9:1|10420225|0|m,s,a,p");
    }

    #[test]
    fn all_profiles_produce_expected_fingerprint() {
        let reg = crate::profile::ProfileRegistry::builtin();
        for browser in [
            ("chrome", 145),
            ("chrome", 146),
            ("chrome", 147),
            ("chrome", 148),
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
                let h2 = H2Config::from_profile(&profile.h2).unwrap();
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
