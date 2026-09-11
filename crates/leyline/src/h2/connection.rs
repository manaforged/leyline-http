use std::collections::VecDeque;
use std::time::Instant;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::h2::client::{self, DriverTask, H2Client};
use crate::h2::config::{H2Config, PseudoOrder, SettingId};
use crate::h2::error::{ErrorCode, H2Error};
use crate::h2::hpack;
use crate::header_str::HeaderStr;

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

pub(crate) struct SettingsApplyResult {
    pub window_size_delta: Option<i64>,
}

impl PeerSettings {
    pub(crate) fn apply(&mut self, params: &[(u16, u32)]) -> Result<SettingsApplyResult, H2Error> {
        let old_window = self.initial_window_size;
        for &(id, val) in params {
            match id {
                0x1 => self.header_table_size = val,
                0x2 => {
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
                _ => {}
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

#[derive(Debug)]
pub struct H2Response {
    pub status: u16,
    pub headers: Vec<(HeaderStr, HeaderStr)>,
    pub body: Vec<u8>,
    pub trailers: Option<Vec<(HeaderStr, HeaderStr)>>,
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct RstFloodDetector {
    events: VecDeque<Instant>,
    threshold: u32,
    window: std::time::Duration,
    label: &'static str,
    reason: &'static str,
}

impl RstFloodDetector {
    pub fn new(threshold: u32, window: std::time::Duration) -> Self {
        Self::with_label(
            threshold,
            window,
            "leyline::h2::rst_flood",
            "peer sent excessive RST_STREAMs",
        )
    }

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

pub struct ClientConnection<T> {
    handle: H2Client,
    _driver: Option<DriverTask>,
    _io_marker: std::marker::PhantomData<T>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> ClientConnection<T> {
    #[tracing::instrument(name = "h2.handshake", level = "debug", skip_all)]
    pub async fn handshake(io: T, config: H2Config) -> Result<Self, H2Error> {
        let (handle, driver) = client::start(io, config).await?;
        Ok(Self {
            handle,
            _driver: Some(driver),
            _io_marker: std::marker::PhantomData,
        })
    }

    pub async fn start(io: T, config: H2Config) -> Result<(H2Client, DriverTask), H2Error> {
        client::start(io, config).await
    }

    pub fn handle(&self) -> &H2Client {
        &self.handle
    }

    pub async fn send_request(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<HeaderPair>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        self.handle.send_request(pseudo, headers, body).await
    }

    pub async fn send_request_with_trailers(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<HeaderPair>,
        body: Option<Bytes>,
        trailers: Vec<(String, String)>,
    ) -> Result<H2Response, H2Error> {
        self.handle
            .send_request_with_trailers(pseudo, headers, body, trailers)
            .await
    }
}

pub(crate) type HeaderPair = (
    std::borrow::Cow<'static, str>,
    std::borrow::Cow<'static, str>,
);

#[derive(Debug, Clone, Default)]
pub struct PseudoHeaders {
    pub method: String,
    pub scheme: String,
    pub authority: String,
    pub path: String,
    pub protocol: Option<String>,
}

impl PseudoHeaders {
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

pub(crate) fn encode_request_pseudos<'a>(
    encoder: &mut hpack::Encoder,
    pseudo_list: Vec<(&'a str, &'a str)>,
    headers: &'a [HeaderPair],
) -> Vec<u8> {
    let count = pseudo_list.len() + headers.len();
    let pairs = pseudo_list
        .iter()
        .map(|&(n, v)| (n, v))
        .chain(headers.iter().map(|(n, v)| (n.as_ref(), v.as_ref())));
    encoder.encode_header_block_iter(pairs, count)
}
