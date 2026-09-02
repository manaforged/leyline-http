//! One handler per inbound control frame type.

use std::time::Instant;

use super::*;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    /// Apply peer SETTINGS, re-scale stream windows, and acknowledge.
    pub(super) async fn on_settings(&mut self, s: SettingsFrame) -> Result<(), H2Error> {
        if s.ack {
            return Ok(());
        }
        self.settings_flood.record(Instant::now())?;
        let result = self.peer_settings.apply(&s.params)?;
        if let Some(delta) = result.window_size_delta {
            for (sid, actor) in self.streams.iter_mut() {
                match checked_window_add(actor.send_window, delta) {
                    Ok(v) => actor.send_window = v,
                    Err(new_win) => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::FlowControlError,
                            reason: format!(
                                "SETTINGS_INITIAL_WINDOW_SIZE delta pushes stream {sid} window to {new_win} (> 2^31-1)"
                            ),
                        });
                    }
                }
            }
        }
        self.peer_snapshot
            .set_max_concurrent_streams(self.peer_settings.max_concurrent_streams);
        self.peer_greeted = true;
        self.drain_pending().await?;
        self.peer_snapshot
            .set_enable_connect_protocol(self.peer_settings.enable_connect_protocol);
        self.writer.write_settings_ack().await?;
        let encoder_cap = (self.peer_settings.header_table_size as usize).min(4096);
        self.encoder.set_max_table_size(encoder_cap);
        Ok(())
    }

    /// Credit the connection or one stream with the WINDOW_UPDATE increment.
    pub(super) async fn on_window(&mut self, w: WindowUpdateFrame) -> Result<(), H2Error> {
        if w.stream_id == 0 {
            self.conn_send_window =
                match checked_window_add(self.conn_send_window, w.increment as i64) {
                    Ok(v) => v,
                    Err(new_win) => {
                        return Err(H2Error::Connection {
                            code: ErrorCode::FlowControlError,
                            reason: format!(
                                "WINDOW_UPDATE would push connection window to {new_win} (> 2^31-1)"
                            ),
                        });
                    }
                };
            return Ok(());
        }
        if let Some(actor) = self.streams.get_mut(&w.stream_id) {
            match checked_window_add(actor.send_window, w.increment as i64) {
                Ok(v) => actor.send_window = v,
                Err(_) => {
                    self.writer
                        .write_rst_stream(w.stream_id, ErrorCode::FlowControlError)
                        .await?;
                    let err = H2Error::Stream {
                        stream_id: w.stream_id,
                        code: ErrorCode::FlowControlError,
                    };
                    self.fail_stream(w.stream_id, err);
                }
            }
        }
        Ok(())
    }

    /// Answer a peer PING; ignore an acknowledgement of our own.
    pub(super) async fn on_ping(&mut self, p: PingFrame) -> Result<(), H2Error> {
        if !p.ack {
            self.writer.write_ping_ack(p.payload).await?;
        }
        Ok(())
    }

    /// Fail every stream above the GOAWAY last-stream id, then close on a real error code.
    pub(super) fn on_goaway(&mut self, g: GoAwayFrame) -> Result<(), H2Error> {
        self.peer_goaway_last_stream = Some(g.last_stream_id);
        let to_fail: Vec<u32> = self
            .streams
            .keys()
            .copied()
            .filter(|sid| *sid > g.last_stream_id)
            .collect();
        if matches!(g.error_code, ErrorCode::NoError) {
            for sid in to_fail {
                self.fail_stream(
                    sid,
                    H2Error::Stream {
                        stream_id: sid,
                        code: ErrorCode::RefusedStream,
                    },
                );
            }
            return Ok(());
        }
        for sid in to_fail {
            self.fail_stream(
                sid,
                H2Error::Connection {
                    code: g.error_code,
                    reason: format!("peer GOAWAY: {:?}", g.error_code),
                },
            );
        }
        Err(H2Error::Connection {
            code: g.error_code,
            reason: format!("server sent GOAWAY: {:?}", g.error_code),
        })
    }

    /// Tear down the stream a peer RST_STREAM names, rejecting idle stream ids.
    pub(super) fn on_rst(&mut self, r: RstStreamFrame) -> Result<(), H2Error> {
        if r.stream_id == 0 || (r.stream_id % 2 == 1 && r.stream_id >= self.next_stream_id) {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: format!("peer sent RST_STREAM on idle stream id {}", r.stream_id),
            });
        }
        self.rst_flood.record(Instant::now())?;
        if let Some(actor) = self.streams.get_mut(&r.stream_id) {
            let _ = actor
                .state
                .transition(StreamEvent::RecvRstStream(r.error_code));
        }
        let err = H2Error::Stream {
            stream_id: r.stream_id,
            code: r.error_code,
        };
        self.fail_stream(r.stream_id, err);
        Ok(())
    }

    /// Refuse server push: cancel the promised stream but keep the HPACK table in sync.
    pub(super) async fn on_push(&mut self, pp: PushPromiseFrame) -> Result<(), H2Error> {
        let decoded = self.decoder.decode_header_block(&pp.fragment);
        self.writer
            .write_rst_stream(pp.promised_stream_id, ErrorCode::Cancel)
            .await?;
        decoded.map_err(H2Error::Hpack)?;
        Ok(())
    }
}
