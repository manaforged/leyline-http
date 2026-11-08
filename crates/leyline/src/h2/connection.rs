use std::collections::VecDeque;
use std::time::Instant;

use crate::h2::error::ErrorCode;

use crate::h2::H2Error;
use crate::h2::frame::{
    DEFAULT_HEADER_TABLE_SIZE, DEFAULT_MAX_FRAME_SIZE, MAX_FRAME_SIZE_LIMIT, U31_MAX,
};
use crate::h2::hpack;
use crate::header_str::HeaderStr;
use crate::profile::{DEFAULT_WINDOW_SIZE, PseudoOrder, SettingId};

#[derive(Debug, Clone)]
pub struct PeerSettings {
    pub header_table_size: u32,
    pub enable_push: bool,
    pub max_concurrent_streams: Option<u32>,
    pub initial_window_size: u32,
    pub max_frame_size: u32,
    pub max_header_list_size: Option<u32>,
    pub enable_connect_protocol: bool,
}

impl Default for PeerSettings {
    fn default() -> Self {
        Self {
            header_table_size: DEFAULT_HEADER_TABLE_SIZE,
            enable_push: true,
            max_concurrent_streams: None,
            initial_window_size: DEFAULT_WINDOW_SIZE,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            max_header_list_size: None,
            enable_connect_protocol: false,
        }
    }
}

pub(crate) struct SettingsApplyResult {
    pub window_size_delta: Option<i64>,
}

impl PeerSettings {
    pub(crate) fn apply(&mut self, params: &[(u16, u32)]) -> Result<SettingsApplyResult, H2Error> {
        let old_window = self.initial_window_size;
        for &(id, val) in params {
            self.set(id, val)?;
        }
        let delta = if self.initial_window_size != old_window {
            Some(self.initial_window_size as i64 - old_window as i64)
        } else {
            None
        };
        Ok(SettingsApplyResult {
            window_size_delta: delta,
        })
    }

    fn set(&mut self, code: u16, val: u32) -> Result<(), H2Error> {
        let Some(id) = SettingId::from_code(code) else {
            return Ok(());
        };
        match id {
            SettingId::HeaderTableSize => self.header_table_size = val,
            SettingId::EnablePush => {
                if val != 0 {
                    return Err(H2Error::Connection {
                        code: ErrorCode::ProtocolError,
                        reason: format!("server sent ENABLE_PUSH {val}; only 0 is allowed"),
                    });
                }
                self.enable_push = false;
            }
            SettingId::MaxConcurrentStreams => self.max_concurrent_streams = Some(val),
            SettingId::InitialWindowSize => {
                if val > U31_MAX {
                    return Err(H2Error::Connection {
                        code: ErrorCode::FlowControlError,
                        reason: "INITIAL_WINDOW_SIZE exceeds max".into(),
                    });
                }
                self.initial_window_size = val;
            }
            SettingId::MaxFrameSize => {
                if !(DEFAULT_MAX_FRAME_SIZE..=MAX_FRAME_SIZE_LIMIT).contains(&val) {
                    return Err(H2Error::Connection {
                        code: ErrorCode::ProtocolError,
                        reason: format!("MAX_FRAME_SIZE {val} out of range"),
                    });
                }
                self.max_frame_size = val;
            }
            SettingId::MaxHeaderListSize => self.max_header_list_size = Some(val),
            SettingId::EnableConnectProtocol => {
                if val > 1 || (self.enable_connect_protocol && val == 0) {
                    return Err(H2Error::Connection {
                        code: ErrorCode::ProtocolError,
                        reason: format!(
                            "ENABLE_CONNECT_PROTOCOL {val} is not 0 or 1, or reverts 1 to 0"
                        ),
                    });
                }
                self.enable_connect_protocol = val == 1;
            }
            SettingId::NoRfc7540Priorities => {}
        }
        Ok(())
    }
}

/// The peer behavior a [`FloodGuard`] limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flood {
    /// RST_STREAM frames.
    RstStream,
    /// SETTINGS frames that are not an ACK.
    Settings,
    /// PING, WINDOW_UPDATE on unknown streams, and empty CONTINUATION frames.
    Control,
}

impl Flood {
    fn label(self) -> &'static str {
        match self {
            Self::RstStream => "leyline::h2::rst_flood",
            Self::Settings => "leyline::h2::settings_flood",
            Self::Control => "leyline::h2::control_flood",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::RstStream => "peer sent excessive RST_STREAMs",
            Self::Settings => "peer sent excessive SETTINGS updates",
            Self::Control => "peer sent excessive PING, WINDOW_UPDATE or empty CONTINUATION frames",
        }
    }
}

/// Counts one kind of peer frame in a sliding window and fails the connection past a threshold.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct FloodGuard {
    events: VecDeque<Instant>,
    threshold: u32,
    window: std::time::Duration,
    flood: Flood,
}

impl FloodGuard {
    /// Builds a guard that allows `threshold` frames of `flood` per `window`.
    pub fn new(flood: Flood, threshold: u32, window: std::time::Duration) -> Self {
        Self {
            events: VecDeque::new(),
            threshold,
            window,
            flood,
        }
    }

    /// Records one frame seen at `at` and fails once the window holds more than the threshold.
    pub fn record(&mut self, at: Instant) -> Result<(), H2Error> {
        while let Some(front) = self.events.front().copied() {
            if at.saturating_duration_since(front) > self.window {
                self.events.pop_front();
            } else {
                break;
            }
        }
        self.events.push_back(at);

        if self.events.len() > self.threshold as usize {
            tracing::warn!(
                target: "leyline::h2::flood_guard",
                detector = self.flood.label(),
                events_in_window = self.events.len(),
                threshold = self.threshold,
                window_ms = self.window.as_millis() as u64,
                "flood guard tripped — tearing down connection with ENHANCE_YOUR_CALM"
            );
            return Err(H2Error::Connection {
                code: ErrorCode::EnhanceYourCalm,
                reason: self.flood.reason().into(),
            });
        }
        Ok(())
    }
}

use crate::wire::HeaderPair;

pub type PseudoList<'a> = ([(&'a str, &'a str); 5], usize);

#[derive(Debug, Clone, Default)]
pub struct PseudoHeaders {
    pub method: HeaderStr,
    pub scheme: HeaderStr,
    pub authority: HeaderStr,
    pub path: HeaderStr,
    pub protocol: Option<HeaderStr>,
}

impl PseudoHeaders {
    /// Returns whether `:method` is `method`, ignoring ASCII case.
    pub fn method_is(&self, method: &http::Method) -> bool {
        self.method.eq_ignore_ascii_case(method.as_str())
    }

    pub fn build_pseudo_list<'a>(
        &'a self,
        pseudo_order: &[PseudoOrder; 4],
    ) -> Result<PseudoList<'a>, H2Error> {
        let connect = self.method_is(&http::Method::CONNECT);
        let is_connect = connect && self.protocol.is_none();
        if connect && self.authority.is_empty() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "CONNECT requires :authority".into(),
            });
        }

        let mut list = [("", ""); 5];
        let mut len = 0;
        let mut push = |n: &'a str, v: &'a str| {
            list[len] = (n, v);
            len += 1;
        };
        for order in pseudo_order {
            match order {
                PseudoOrder::Method => push(":method", self.method.as_str()),
                PseudoOrder::Scheme if !is_connect => push(":scheme", self.scheme.as_str()),
                PseudoOrder::Scheme => {}
                PseudoOrder::Authority => push(":authority", self.authority.as_str()),
                PseudoOrder::Path if !is_connect => push(":path", self.path.as_str()),
                PseudoOrder::Path => {}
            }
        }
        if let Some(proto) = self.protocol.as_ref().map(|p| p.as_str()) {
            push(":protocol", proto);
        }
        Ok((list, len))
    }
}

pub(crate) fn encode_request_pseudos<'a, 'e>(
    encoder: &'e mut hpack::Encoder,
    pseudo_list: &[(&'a str, &'a str)],
    headers: &'a [HeaderPair],
) -> &'e [u8] {
    let pairs = pseudo_list
        .iter()
        .copied()
        .chain(headers.iter().map(|(n, v)| (n.as_ref(), v.as_ref())));
    encoder.encode_block(pairs)
}

#[cfg(kani)]
#[path = "connection_proofs.rs"]
mod proofs;
