//! HTTP/2 client connection — handshake, settings exchange, stream dispatch.
//!
//! This module retains the legacy `ClientConnection` API as a thin shell
//! around the concurrent driver living in [`crate::h2::client`]. New code
//! should prefer [`crate::h2::client::H2Client`] directly — it is cloneable
//! and multiplexes concurrent requests over one TCP connection without
//! head-of-line blocking. `ClientConnection` remains for backward
//! compatibility with tests and callers that want a single-owner handle.

use std::collections::VecDeque;
use std::time::Instant;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::client::{self, DriverTask, H2Client};
use crate::h2::config::{H2Config, PseudoOrder, SettingId};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::hpack;
use crate::h2::stream_state::StreamState;

/// Peer SETTINGS (received from server).
#[derive(Debug, Clone)]
pub struct PeerSettings {
    /// SETTINGS_HEADER_TABLE_SIZE advertised by the peer (in octets).
    pub header_table_size: u32,
    /// SETTINGS_ENABLE_PUSH — whether the peer accepts server push.
    pub enable_push: bool,
    /// SETTINGS_MAX_CONCURRENT_STREAMS — peer's concurrency limit, if any.
    pub max_concurrent_streams: Option<u32>,
    /// SETTINGS_INITIAL_WINDOW_SIZE — per-stream flow-control window.
    pub initial_window_size: u32,
    /// SETTINGS_MAX_FRAME_SIZE — the largest frame the peer will accept.
    pub max_frame_size: u32,
    /// SETTINGS_MAX_HEADER_LIST_SIZE — peer's header list size limit, if any.
    pub max_header_list_size: Option<u32>,
    /// RFC 8441 §3 — `SETTINGS_ENABLE_CONNECT_PROTOCOL`. When `true` the
    /// peer accepts an extended CONNECT request carrying a `:protocol`
    /// pseudo-header (e.g. WebSocket over HTTP/2). Default `false`;
    /// the setting is sticky per RFC 8441 §3 — once `1` it cannot be
    /// reverted to `0`.
    pub enable_connect_protocol: bool,
}

impl Default for PeerSettings {
    fn default() -> Self {
        Self {
            header_table_size: 4096,
            enable_push: true,
            max_concurrent_streams: None,
            initial_window_size: 65535,
            max_frame_size: 16384,
            max_header_list_size: None,
            enable_connect_protocol: false,
        }
    }
}

/// Result of applying SETTINGS — carries deltas for flow control adjustment.
pub(crate) struct SettingsApplyResult {
    /// Change in INITIAL_WINDOW_SIZE (new - old), if it changed.
    pub window_size_delta: Option<i64>,
}

impl PeerSettings {
    pub(crate) fn apply(&mut self, params: &[(u16, u32)]) -> Result<SettingsApplyResult, H2Error> {
        let old_window = self.initial_window_size;
        for &(id, val) in params {
            match id {
                0x1 => self.header_table_size = val,
                0x2 => {
                    // RFC 9113 Section 6.5.2: must be 0 or 1.
                    if val > 1 {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: format!("ENABLE_PUSH value {val} is not 0 or 1"),
                        });
                    }
                    self.enable_push = val != 0;
                }
                0x3 => self.max_concurrent_streams = Some(val),
                0x4 => {
                    if val > 0x7FFF_FFFF {
                        return Err(H2Error::Connection {
                            code: ErrorCode::FlowControlError,
                            reason: "INITIAL_WINDOW_SIZE exceeds max".into(),
                        });
                    }
                    self.initial_window_size = val;
                }
                0x5 => {
                    if !(16384..=16777215).contains(&val) {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: format!("MAX_FRAME_SIZE {val} out of range"),
                        });
                    }
                    self.max_frame_size = val;
                }
                0x6 => self.max_header_list_size = Some(val),
                0x8 => {
                    // RFC 8441 §3 — SETTINGS_ENABLE_CONNECT_PROTOCOL.
                    // Value must be 0 or 1; once advertised as 1 the
                    // peer MUST NOT revert to 0 on the same connection.
                    if val > 1 {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: format!("ENABLE_CONNECT_PROTOCOL value {val} is not 0 or 1"),
                        });
                    }
                    if self.enable_connect_protocol && val == 0 {
                        return Err(H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "ENABLE_CONNECT_PROTOCOL cannot be reverted to 0".into(),
                        });
                    }
                    self.enable_connect_protocol = val == 1;
                }
                _ => {} // Unknown settings MUST be ignored (RFC 9113 6.5.2)
            }
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
}

/// A response received from the server.
#[derive(Debug)]
pub struct H2Response {
    /// HTTP status code.
    pub status: u16,
    /// Response headers (name, value pairs in order).
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
    /// Trailer headers, if any.
    pub trailers: Option<Vec<(String, String)>>,
}

/// Per-stream bookkeeping: RFC 9113 state plus flow-control windows.
///
/// Retained for API compatibility with callers that inspect stream info;
/// the active driver keeps its own equivalent state internally.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Current stream state per the RFC 9113 §5.1 state machine.
    pub state: StreamState,
    /// Our send-side flow-control window for this stream.
    pub send_window: i64,
    /// Our recv-side flow-control window for this stream.
    pub recv_window: i64,
}

/// Sliding-window flood detector shared by the RST_STREAM and
/// SETTINGS defences.
///
/// Pulled out of `ClientConnection` so the state can be exercised by
/// pure integration tests without standing up a real IO stream.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct RstFloodDetector {
    events: VecDeque<Instant>,
    threshold: u32,
    window: std::time::Duration,
    /// Tracing `target` so RST vs SETTINGS trips are distinguishable
    /// in operator logs without separate detector types.
    label: &'static str,
    /// Human-readable reason attached to the `EnhanceYourCalm`
    /// connection error on trip.
    reason: &'static str,
}

impl RstFloodDetector {
    /// Build a detector with the given sliding window + threshold
    /// and a generic "RST_STREAM" label.
    pub fn new(threshold: u32, window: std::time::Duration) -> Self {
        Self::with_label(
            threshold,
            window,
            "leyline::h2::rst_flood",
            "peer sent excessive RST_STREAMs",
        )
    }

    /// Build a detector with a custom label + reason. Used by the
    /// SETTINGS flood guard so it can surface as a distinct
    /// `leyline::h2::settings_flood` tracing target.
    pub fn with_label(
        threshold: u32,
        window: std::time::Duration,
        label: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            events: VecDeque::new(),
            threshold,
            window,
            label,
            reason,
        }
    }

    /// Record one event at `at`; return `Err(EnhanceYourCalm)` if the
    /// window now holds strictly more than `threshold` events.
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
                detector = self.label,
                events_in_window = self.events.len(),
                threshold = self.threshold,
                window_ms = self.window.as_millis() as u64,
                "flood guard tripped — tearing down connection with ENHANCE_YOUR_CALM"
            );
            return Err(H2Error::Connection {
                code: ErrorCode::EnhanceYourCalm,
                reason: self.reason.into(),
            });
        }
        Ok(())
    }
}

/// Legacy HTTP/2 client connection shell.
///
/// Wraps a running [`H2Client`] + [`DriverTask`] behind the pre-driver
/// API (`handshake`, `send_request`, `send_request_with_trailers`). The
/// driver task runs until either this shell is dropped (which in turn
/// drops the single handle it holds — graceful shutdown) or the peer
/// tears the connection down.
pub struct ClientConnection<T> {
    handle: H2Client,
    /// Kept so the driver task is aborted when the shell is dropped;
    /// prevents orphaned tasks when callers don't explicitly shut down.
    _driver: Option<DriverTask>,
    _io_marker: std::marker::PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> ClientConnection<T> {
    /// Perform the HTTP/2 handshake with fingerprint-accurate SETTINGS.
    #[tracing::instrument(name = "h2.handshake", level = "debug", skip_all)]
    pub async fn handshake(io: T, config: H2Config) -> Result<Self, H2Error> {
        let (handle, driver) = client::start(io, config).await?;
        Ok(Self {
            handle,
            _driver: Some(driver),
            _io_marker: std::marker::PhantomData,
        })
    }

    /// Start a concurrent multiplexing client over `io`, returning the
    /// cloneable handle and the driver task. Prefer this for new code.
    pub async fn start(io: T, config: H2Config) -> Result<(H2Client, DriverTask), H2Error> {
        client::start(io, config).await
    }

    /// Borrow the underlying cloneable handle. Useful for callers that
    /// want to issue concurrent requests without going through the
    /// single-owner `send_request` API.
    pub fn handle(&self) -> &H2Client {
        &self.handle
    }

    /// Send a request and receive the full response.
    pub async fn send_request(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        self.handle.send_request(pseudo, headers, body).await
    }

    /// Send a request with optional trailers, receive the full response.
    ///
    /// When `trailers` is empty this behaves identically to
    /// [`Self::send_request`]. When non-empty, the HEADERS / DATA frames
    /// carry `end_stream = false` and a second HEADERS frame encoding the
    /// trailer block closes the stream (RFC 9113 §8.1).
    pub async fn send_request_with_trailers(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
    ) -> Result<H2Response, H2Error> {
        self.handle
            .send_request_with_trailers(pseudo, headers, body, trailers)
            .await
    }
}

/// Pseudo-headers for a request.
#[derive(Debug, Clone, Default)]
pub struct PseudoHeaders {
    /// `:method` pseudo-header value (HTTP method in upper-case).
    pub method: String,
    /// `:scheme` pseudo-header value (typically `https`).
    pub scheme: String,
    /// `:authority` pseudo-header value (host[:port]).
    pub authority: String,
    /// `:path` pseudo-header value, including the query string.
    pub path: String,
    /// RFC 8441 `:protocol` pseudo-header value for extended CONNECT
    /// (e.g. `Some("websocket")` for WebSocket-over-HTTP/2). `None` for
    /// all other request shapes, including classic CONNECT tunnels.
    pub protocol: Option<String>,
}

impl PseudoHeaders {
    /// Build the ordered pseudo-header list for a request.
    ///
    /// CONNECT (RFC 9113 §8.5) omits `:scheme` and `:path`. Extended
    /// CONNECT (RFC 8441) appends `:protocol` after the classic pseudos.
    /// Returns `Err` if `:method == CONNECT` but `:authority` is empty.
    pub fn build_pseudo_list<'a>(
        &'a self,
        pseudo_order: &[PseudoOrder; 4],
    ) -> Result<Vec<(&'a str, &'a str)>, H2Error> {
        let is_connect = self.method.eq_ignore_ascii_case("CONNECT");
        if is_connect && self.authority.is_empty() {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "CONNECT requires :authority".into(),
            });
        }

        let mut list: Vec<(&str, &str)> = Vec::with_capacity(5);
        for order in pseudo_order {
            match order {
                PseudoOrder::Method => list.push((":method", &self.method)),
                PseudoOrder::Scheme if !is_connect => list.push((":scheme", &self.scheme)),
                PseudoOrder::Scheme => {}
                PseudoOrder::Authority => list.push((":authority", &self.authority)),
                PseudoOrder::Path if !is_connect => list.push((":path", &self.path)),
                PseudoOrder::Path => {}
            }
        }
        if let Some(proto) = self.protocol.as_deref() {
            list.push((":protocol", proto));
        }
        Ok(list)
    }
}

/// Map a [`SettingId`] to its wire-format u16 identifier.
pub(crate) fn id_to_u16(id: &SettingId) -> u16 {
    match id {
        SettingId::HeaderTableSize => 0x1,
        SettingId::EnablePush => 0x2,
        SettingId::MaxConcurrentStreams => 0x3,
        SettingId::InitialWindowSize => 0x4,
        SettingId::MaxFrameSize => 0x5,
        SettingId::MaxHeaderListSize => 0x6,
        SettingId::Unknown8 => 0x8,
        SettingId::Unknown9 => 0x9,
    }
}

/// Encode the pseudo-header list followed by regular request headers
/// into an HPACK header block. Shared between the driver and any
/// backward-compat path that needs a ready-to-ship fragment.
///
/// Takes ownership of nothing — both `pseudo_list` and `headers` must
/// outlive the call. Internally we copy into a scratch `Vec` so the
/// input lifetimes don't have to be unified by the caller.
pub(crate) fn encode_request_pseudos<'a>(
    encoder: &mut hpack::Encoder,
    pseudo_list: Vec<(&'a str, &'a str)>,
    headers: &'a [(String, String)],
) -> Vec<u8> {
    let mut combined: Vec<(&str, &str)> = Vec::with_capacity(pseudo_list.len() + headers.len());
    for (n, v) in &pseudo_list {
        combined.push((*n, *v));
    }
    for (name, value) in headers {
        combined.push((name.as_str(), value.as_str()));
    }
    encoder.encode_header_block(&combined)
}
