//! HTTP/2 client connection — handshake, settings exchange, stream dispatch.
//!
//! Manages the lifecycle of an HTTP/2 connection from preface through
//! request/response to shutdown. Fingerprint config (SETTINGS order,
//! pseudo-header order, window sizes) is applied during handshake.

use std::collections::{HashMap, VecDeque};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::codec::{FrameReader, FrameWriter};
use crate::config::{H2Config, PseudoOrder, SettingId};
use crate::error::{ErrorCode, H2Error};
use crate::frame::*;
use crate::hpack;

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
        }
    }
}

/// Result of applying SETTINGS — carries deltas for flow control adjustment.
pub(crate) struct SettingsApplyResult {
    /// Change in INITIAL_WINDOW_SIZE (new - old), if it changed.
    pub window_size_delta: Option<i64>,
}

impl PeerSettings {
    fn apply(&mut self, params: &[(u16, u32)]) -> Result<SettingsApplyResult, H2Error> {
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

/// HTTP/2 client connection.
pub struct ClientConnection<T> {
    reader: FrameReader<tokio::io::ReadHalf<T>>,
    writer: FrameWriter<tokio::io::WriteHalf<T>>,
    encoder: hpack::Encoder,
    decoder: hpack::Decoder,
    peer_settings: PeerSettings,
    /// Our connection-level send window.
    conn_send_window: i64,
    /// Our connection-level recv window.
    conn_recv_window: i64,
    /// Per-stream send windows.
    stream_windows: HashMap<u32, i64>,
    /// Next client stream ID (odd numbers: 1, 3, 5...).
    next_stream_id: u32,
    /// Config for fingerprinting.
    config: H2Config,
    /// Frames buffered during flow control waits (must be processed by read_response).
    buffered_frames: VecDeque<Frame>,
}

impl<T: AsyncRead + AsyncWrite + Unpin> ClientConnection<T> {
    /// Perform the HTTP/2 handshake with fingerprint-accurate SETTINGS.
    #[tracing::instrument(name = "h2.handshake", level = "debug", skip_all)]
    pub async fn handshake(io: T, config: H2Config) -> Result<Self, H2Error> {
        let (read_half, write_half) = tokio::io::split(io);
        let mut reader = FrameReader::new(read_half);
        let mut writer = FrameWriter::new(write_half);

        // 1. Send client connection preface (RFC 9113 Section 3.4).
        writer.write_preface().await?;

        // 2. Send our SETTINGS frame (ordered per fingerprint config).
        let settings_frame = SettingsFrame {
            ack: false,
            params: config
                .settings
                .iter()
                .map(|(id, val)| (id_to_u16(id), *val))
                .collect(),
        };
        writer.write_settings(&settings_frame).await?;

        // 3. Send WINDOW_UPDATE if connection window > default 65535.
        let default_window: u32 = 65535;
        if config.initial_connection_window_size > default_window {
            let increment = config.initial_connection_window_size - default_window;
            writer
                .write_window_update(&WindowUpdateFrame {
                    stream_id: 0,
                    increment,
                })
                .await?;
        }
        writer.flush().await?;

        // 4. Read server's SETTINGS.
        let mut peer_settings = PeerSettings::default();
        let mut got_settings = false;
        let mut initial_send_window: i64 = 65535;

        // The server may send SETTINGS, WINDOW_UPDATE, or other frames.
        // We need to process until we get SETTINGS and send ACK.
        while !got_settings {
            let frame = reader.next().await?.ok_or_else(|| H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "connection closed before SETTINGS".into(),
            })?;

            match frame {
                Frame::Settings(s) if !s.ack => {
                    let _result = peer_settings.apply(&s.params)?;
                    // No streams exist yet, so no window adjustments needed.
                    writer.write_settings_ack().await?;
                    writer.flush().await?;
                    got_settings = true;
                    // (Our encoder table size is governed by our own SETTINGS.)
                }
                Frame::Settings(s) if s.ack => {
                    // ACK of our SETTINGS — good.
                }
                Frame::WindowUpdate(w) if w.stream_id == 0 => {
                    initial_send_window += w.increment as i64;
                }
                Frame::WindowUpdate(_) => {
                    // Stream-level window update during handshake — ignore.
                }
                Frame::GoAway(g) => {
                    return Err(H2Error::Connection {
                        code: g.error_code,
                        reason: format!("server sent GOAWAY during handshake: {:?}", g.error_code),
                    });
                }
                _ => {
                    // Ignore other frames during handshake.
                }
            }
        }

        // Update max frame size for our reader.
        reader.set_max_frame_size(peer_settings.max_frame_size);

        let conn_window = config.initial_connection_window_size as i64;

        Ok(Self {
            reader,
            writer,
            encoder: hpack::Encoder::new(),
            decoder: {
                let mut d = hpack::Decoder::new();
                // Use our own max_header_list_size (262KB for Chrome 147).
                let max_hdr = config
                    .settings
                    .iter()
                    .find(|(id, _)| matches!(id, crate::config::SettingId::MaxHeaderListSize))
                    .map(|(_, v)| *v as usize)
                    .unwrap_or(256 * 1024);
                d.set_max_header_list_size(max_hdr);
                // Accept dynamic table size updates from server's SETTINGS.
                d.set_max_table_size(peer_settings.header_table_size as usize);
                d
            },
            peer_settings,
            conn_send_window: initial_send_window,
            conn_recv_window: conn_window,
            stream_windows: HashMap::new(),
            next_stream_id: 1,
            config,
            buffered_frames: VecDeque::new(),
        })
    }

    /// Send a request and receive the full response.
    ///
    /// `headers` are (name, value) pairs in the order they should appear.
    /// Pseudo-headers (:method, :scheme, :authority, :path) are reordered
    /// per the fingerprint config.
    pub async fn send_request(
        &mut self,
        pseudo: PseudoHeaders,
        headers: Vec<(String, String)>,
        body: Option<Bytes>,
    ) -> Result<H2Response, H2Error> {
        let stream_id = self.next_stream_id;
        if stream_id > 0x7FFF_FFFF {
            return Err(H2Error::Connection {
                code: ErrorCode::NoError,
                reason: "stream ID space exhausted".into(),
            });
        }
        self.next_stream_id = stream_id + 2;

        // Build the header list with pseudo-headers in fingerprint order.
        let mut header_list: Vec<(&str, &str)> = Vec::new();

        // Add pseudo-headers in configured order.
        for order in &self.config.pseudo_order {
            match order {
                PseudoOrder::Method => header_list.push((":method", &pseudo.method)),
                PseudoOrder::Scheme => header_list.push((":scheme", &pseudo.scheme)),
                PseudoOrder::Authority => header_list.push((":authority", &pseudo.authority)),
                PseudoOrder::Path => header_list.push((":path", &pseudo.path)),
            }
        }

        // Add regular headers.
        for (name, value) in &headers {
            header_list.push((name, value));
        }

        // HPACK encode.
        let fragment = self.encoder.encode_header_block(&header_list);
        let end_stream = body.is_none();
        let max_frame = self.peer_settings.max_frame_size as usize;

        // Split into HEADERS + CONTINUATION if fragment exceeds max frame size.
        if fragment.len() <= max_frame {
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: true,
                    priority: None,
                    fragment: Bytes::from(fragment),
                })
                .await?;
        } else {
            // First chunk in HEADERS frame.
            let first = &fragment[..max_frame];
            self.writer
                .write_headers(&HeadersFrame {
                    stream_id,
                    end_stream,
                    end_headers: false,
                    priority: None,
                    fragment: Bytes::copy_from_slice(first),
                })
                .await?;

            // Remaining chunks as CONTINUATION frames.
            let mut offset = max_frame;
            while offset < fragment.len() {
                let end = (offset + max_frame).min(fragment.len());
                let is_last = end == fragment.len();
                let chunk = &fragment[offset..end];

                let mut buf = BytesMut::with_capacity(9 + chunk.len());
                let header = crate::frame::FrameHeader {
                    length: chunk.len() as u32,
                    frame_type: 0x9,                      // CONTINUATION
                    flags: if is_last { 0x4 } else { 0 }, // END_HEADERS
                    stream_id,
                };
                header.encode(&mut buf);
                buf.extend_from_slice(chunk);
                self.writer.write_raw(&buf).await?;

                offset = end;
            }
        }

        // Send body if present, respecting flow control.
        if let Some(data) = body {
            let max_frame = self.peer_settings.max_frame_size as usize;
            let initial_window = self.peer_settings.initial_window_size as usize;
            let total = data.len();
            let mut sent = 0;

            while sent < total {
                // Respect connection and stream send windows.
                let conn_avail = self.conn_send_window.max(0) as usize;
                let stream_avail = self
                    .stream_windows
                    .get(&stream_id)
                    .copied()
                    .unwrap_or(initial_window as i64)
                    .max(0) as usize;

                let window = conn_avail.min(stream_avail);
                let chunk_size = (total - sent).min(max_frame).min(window.max(1));

                // If window is exhausted, read frames until we get WINDOW_UPDATE.
                // Buffer any non-control frames (HEADERS, DATA, etc.) for read_response().
                if window == 0 && sent < total {
                    let frame = self
                        .reader
                        .next()
                        .await?
                        .ok_or_else(|| H2Error::Connection {
                            code: ErrorCode::ProtocolError,
                            reason: "connection closed while waiting for WINDOW_UPDATE".into(),
                        })?;
                    match frame {
                        Frame::WindowUpdate(w) if w.stream_id == 0 => {
                            self.conn_send_window += w.increment as i64;
                        }
                        Frame::WindowUpdate(w) if w.stream_id == stream_id => {
                            *self.stream_windows.entry(stream_id).or_insert(0) +=
                                w.increment as i64;
                        }
                        Frame::Settings(s) if !s.ack => {
                            let result = self.peer_settings.apply(&s.params)?;
                            self.apply_settings_delta(&result);
                            self.writer.write_settings_ack().await?;
                            self.decoder
                                .set_max_table_size(self.peer_settings.header_table_size as usize);
                            self.encoder
                                .set_max_table_size(self.peer_settings.header_table_size as usize);
                        }
                        Frame::Settings(s) if s.ack => {} // ACK of our settings
                        Frame::Ping(p) if !p.ack => {
                            self.writer.write_ping_ack(p.payload).await?;
                        }
                        Frame::GoAway(g) => {
                            return Err(H2Error::Connection {
                                code: g.error_code,
                                reason: format!("GOAWAY during send: {:?}", g.error_code),
                            });
                        }
                        other => {
                            // Buffer for read_response() — don't drop HEADERS/DATA.
                            self.buffered_frames.push_back(other);
                        }
                    }
                    continue;
                }

                let end = sent + chunk_size;
                let is_last = end >= total;
                let chunk = data.slice(sent..end);

                self.writer
                    .write_data(&DataFrame {
                        stream_id,
                        end_stream: is_last,
                        data: chunk,
                    })
                    .await?;

                self.conn_send_window -= chunk_size as i64;
                *self
                    .stream_windows
                    .entry(stream_id)
                    .or_insert(initial_window as i64) -= chunk_size as i64;
                sent = end;
            }
        } else {
            // No body — HEADERS already had end_stream: true.
        }

        self.writer.flush().await?;

        // RFC 9113 §8.3.1: a server MUST NOT generate a response body
        // for a HEAD request. Many real-world servers misbehave and
        // send DATA frames anyway; we pragmatically accept them and
        // drop the bytes on the floor rather than RST_STREAM'ing.
        let is_head = pseudo.method.eq_ignore_ascii_case("HEAD");

        // Read response.
        self.read_response(stream_id, is_head).await
    }

    /// Maximum response body size (100 MB).
    const MAX_BODY_SIZE: usize = 100 * 1024 * 1024;

    /// Get the next frame — drain buffered frames first, then read from wire.
    async fn next_frame(&mut self) -> Result<Option<Frame>, H2Error> {
        if let Some(frame) = self.buffered_frames.pop_front() {
            return Ok(Some(frame));
        }
        self.reader.next().await
    }

    /// Apply SETTINGS delta to existing stream send windows (RFC 9113 Section 6.5.2).
    fn apply_settings_delta(&mut self, result: &SettingsApplyResult) {
        if let Some(delta) = result.window_size_delta {
            for window in self.stream_windows.values_mut() {
                *window += delta;
            }
        }
    }

    /// Read a complete response for a stream.
    ///
    /// When `drop_body` is true (e.g. HEAD requests), DATA frames are
    /// still consumed from the wire — flow control demands it — but
    /// their payloads are discarded instead of being buffered into
    /// `body`. We additionally flip to drop-body mode on the fly when
    /// the status line comes back as 1xx/204/304, mirroring the H1
    /// transport's rule and matching RFC 9110 §6.4.1.
    async fn read_response(
        &mut self,
        stream_id: u32,
        drop_body: bool,
    ) -> Result<H2Response, H2Error> {
        let mut status = 0u16;
        let mut resp_headers = Vec::new();
        let mut body = Vec::new();
        let mut trailers = None;
        let mut got_headers = false;
        // Start from the method-based hint; may be upgraded to true
        // after the HEADERS frame reveals a body-less status code.
        let mut drop_body = drop_body;

        loop {
            let frame = self
                .next_frame()
                .await?
                .ok_or_else(|| H2Error::Connection {
                    code: ErrorCode::ProtocolError,
                    reason: "connection closed while reading response".into(),
                })?;

            match frame {
                Frame::Headers(h) if h.stream_id == stream_id => {
                    // Reassemble CONTINUATION fragments if needed.
                    let full_fragment =
                        if h.end_headers {
                            h.fragment
                        } else {
                            const MAX_HEADER_BLOCK: usize = 64 * 1024;
                            let mut assembled = h.fragment.to_vec();
                            loop {
                                if assembled.len() > MAX_HEADER_BLOCK {
                                    return Err(H2Error::Connection {
                                        code: ErrorCode::CompressionError,
                                        reason: "header block too large".into(),
                                    });
                                }
                                let cont = self.reader.next().await?.ok_or_else(|| {
                                    H2Error::Connection {
                                        code: ErrorCode::ProtocolError,
                                        reason: "connection closed during CONTINUATION".into(),
                                    }
                                })?;
                                match cont {
                                    Frame::Continuation {
                                        stream_id: sid,
                                        end_headers,
                                        fragment,
                                    } if sid == stream_id => {
                                        assembled.extend_from_slice(&fragment);
                                        if end_headers {
                                            break;
                                        }
                                    }
                                    _ => {
                                        return Err(H2Error::Connection {
                                            code: ErrorCode::ProtocolError,
                                            reason: "expected CONTINUATION frame".into(),
                                        });
                                    }
                                }
                            }
                            Bytes::from(assembled)
                        };

                    let decoded = self
                        .decoder
                        .decode_header_block(&full_fragment)
                        .map_err(|e| H2Error::Hpack(e))?;

                    if !got_headers {
                        // Response headers.
                        for header in &decoded {
                            if header.name == ":status" {
                                status = header
                                    .value
                                    .parse()
                                    .map_err(|_| H2Error::Hpack("invalid :status".into()))?;
                            } else if !header.name.starts_with(':') {
                                resp_headers.push((header.name.clone(), header.value.clone()));
                            }
                        }
                        got_headers = true;

                        // Status codes that cannot carry a body per
                        // RFC 9110 §6.4.1. Flip drop_body so any stray
                        // DATA frames the server sends are discarded.
                        if matches!(status, 100..=199 | 204 | 304) {
                            drop_body = true;
                        }

                        if h.end_stream {
                            break;
                        }
                    } else {
                        // Trailers.
                        let mut trailer_headers = Vec::new();
                        for header in &decoded {
                            trailer_headers.push((header.name.clone(), header.value.clone()));
                        }
                        trailers = Some(trailer_headers);
                        break;
                    }
                }
                Frame::Data(d) if d.stream_id == stream_id => {
                    // Drop the bytes for HEAD / 1xx / 204 / 304 but
                    // still account for them in flow control below
                    // (dropping them silently would eventually stall
                    // the connection if a misbehaving server keeps
                    // sending DATA on the same session).
                    if !drop_body {
                        if body.len() + d.data.len() > Self::MAX_BODY_SIZE {
                            return Err(H2Error::Connection {
                                code: ErrorCode::Cancel,
                                reason: format!(
                                    "response body exceeds {} bytes",
                                    Self::MAX_BODY_SIZE
                                ),
                            });
                        }
                        body.extend_from_slice(&d.data);
                    }

                    // Track consumed window and send WINDOW_UPDATE when half depleted.
                    // Chrome batches window updates instead of sending per-frame.
                    let len = d.data.len() as i64;
                    self.conn_recv_window -= len;
                    let stream_window = self
                        .stream_windows
                        .entry(stream_id)
                        .or_insert(self.peer_settings.initial_window_size as i64);
                    *stream_window -= len;

                    let initial_conn = self.config.initial_connection_window_size as i64;
                    let initial_stream = self.peer_settings.initial_window_size as i64;

                    // Send connection WINDOW_UPDATE when below half.
                    if self.conn_recv_window < initial_conn / 2 {
                        let increment = (initial_conn - self.conn_recv_window)
                            .max(1)
                            .min(0x7FFF_FFFF) as u32;
                        if increment > 0 {
                            self.writer
                                .write_window_update(&WindowUpdateFrame {
                                    stream_id: 0,
                                    increment,
                                })
                                .await?;
                            self.conn_recv_window += increment as i64;
                        }
                    }

                    // Send stream WINDOW_UPDATE when below half.
                    if *stream_window < initial_stream / 2 {
                        let increment =
                            (initial_stream - *stream_window).max(1).min(0x7FFF_FFFF) as u32;
                        if increment > 0 {
                            self.writer
                                .write_window_update(&WindowUpdateFrame {
                                    stream_id,
                                    increment,
                                })
                                .await?;
                            *stream_window += increment as i64;
                        }
                    }

                    if d.end_stream {
                        break;
                    }
                }
                Frame::Settings(s) if s.ack => {
                    // ACK of our settings — expected.
                }
                Frame::Settings(s) if !s.ack => {
                    // Server sent new SETTINGS mid-connection.
                    let result = self.peer_settings.apply(&s.params)?;
                    self.apply_settings_delta(&result);
                    self.writer.write_settings_ack().await?;
                    self.reader
                        .set_max_frame_size(self.peer_settings.max_frame_size);
                    // Update HPACK table sizes.
                    self.decoder
                        .set_max_table_size(self.peer_settings.header_table_size as usize);
                    // Encoder table size governed by server's HEADER_TABLE_SIZE
                    // (signals in next header block per RFC 7541 Section 4.2).
                    self.encoder
                        .set_max_table_size(self.peer_settings.header_table_size as usize);
                }
                Frame::WindowUpdate(w) if w.stream_id == 0 => {
                    self.conn_send_window += w.increment as i64;
                }
                Frame::WindowUpdate(w) if w.stream_id == stream_id => {
                    *self.stream_windows.entry(stream_id).or_insert(65535) += w.increment as i64;
                }
                Frame::Ping(p) if !p.ack => {
                    // Respond to PING.
                    self.writer.write_ping_ack(p.payload).await?;
                }
                Frame::GoAway(g) => {
                    return Err(H2Error::Connection {
                        code: g.error_code,
                        reason: format!("server sent GOAWAY: {:?}", g.error_code),
                    });
                }
                Frame::RstStream(r) if r.stream_id == stream_id => {
                    return Err(H2Error::Stream {
                        stream_id,
                        code: r.error_code,
                    });
                }
                Frame::PushPromise(pp) => {
                    // RST_STREAM the promised stream immediately (Chrome behavior).
                    self.writer
                        .write_rst_stream(pp.promised_stream_id, ErrorCode::Cancel)
                        .await?;
                }
                _ => {
                    // Ignore other frames for other streams / unknown types.
                }
            }
        }

        self.writer.flush().await?;

        // Clean up stream state to prevent unbounded growth.
        self.stream_windows.remove(&stream_id);

        Ok(H2Response {
            status,
            headers: resp_headers,
            body,
            trailers,
        })
    }
}

/// Pseudo-headers for a request.
#[derive(Debug, Clone)]
pub struct PseudoHeaders {
    /// `:method` pseudo-header value (HTTP method in upper-case).
    pub method: String,
    /// `:scheme` pseudo-header value (typically `https`).
    pub scheme: String,
    /// `:authority` pseudo-header value (host[:port]).
    pub authority: String,
    /// `:path` pseudo-header value, including the query string.
    pub path: String,
}

fn id_to_u16(id: &SettingId) -> u16 {
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
