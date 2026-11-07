use std::time::Instant;

use crate::h2::config::SettingId;

use super::*;

const UNSET_MAX_CONCURRENT_STREAMS: usize = 1000;

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) fn on_settings(&mut self, s: SettingsFrame) -> Result<(), H2Error> {
        if s.ack {
            self.local_settings_acked = true;
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
        self.peer_greeted = true;
        self.load.capacity.store(
            self.peer_settings
                .max_concurrent_streams
                .map_or(UNSET_MAX_CONCURRENT_STREAMS, |limit| limit as usize),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.peer_snapshot
            .set_enable_connect_protocol(self.peer_settings.enable_connect_protocol);
        self.writer.write_frame(&SettingsFrame::ack())?;
        let encoder_cap = (self.peer_settings.header_table_size as usize)
            .min(crate::h2::config::DEFAULT_HEADER_TABLE_SIZE as usize);
        self.encoder.set_max_table_size(encoder_cap);
        Ok(())
    }

    pub(super) fn on_window(&mut self, w: WindowUpdateFrame) -> Result<(), H2Error> {
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
        if let Some(actor) = self.streams.get_mut(w.stream_id) {
            if actor.state.is_reset_local() || actor.remote_done {
                return Ok(());
            }
            match checked_window_add(actor.send_window, w.increment as i64) {
                Ok(v) => actor.send_window = v,
                Err(_) => {
                    self.writer.write_frame(&RstStreamFrame {
                        stream_id: w.stream_id,
                        error_code: ErrorCode::FlowControlError,
                    })?;
                    let err = H2Error::Stream {
                        stream_id: w.stream_id,
                        code: ErrorCode::FlowControlError,
                    };
                    self.fail_stream(w.stream_id, err);
                }
            }
        } else {
            self.control_flood.record(Instant::now())?;
        }
        Ok(())
    }

    pub(super) fn on_ping(&mut self, p: PingFrame) -> Result<(), H2Error> {
        if !p.ack {
            self.control_flood.record(Instant::now())?;
            self.writer.write_frame(&PingFrame {
                ack: true,
                payload: p.payload,
            })?;
            return Ok(());
        }
        if let Some(index) = self
            .pings
            .iter()
            .position(|(payload, ..)| *payload == p.payload)
            && let Some((_, ack_tx, _)) = self.pings.remove(index)
            && ack_tx.send(()).is_err()
        {
            tracing::trace!("ping ack receiver dropped");
        }
        Ok(())
    }

    pub(super) fn on_goaway(&mut self, g: GoAwayFrame) -> Result<(), H2Error> {
        self.peer_goaway_last_stream = Some(g.last_stream_id);
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        let to_fail: Vec<u32> = self
            .streams
            .keys()
            .copied()
            .filter(|sid| *sid > g.last_stream_id)
            .collect();
        for sid in to_fail {
            self.fail_stream(
                sid,
                H2Error::Stream {
                    stream_id: sid,
                    code: ErrorCode::RefusedStream,
                },
            );
        }
        if matches!(g.error_code, ErrorCode::NoError) {
            return Ok(());
        }
        Err(H2Error::Connection {
            code: g.error_code,
            reason: format!("server sent GOAWAY: {:?}", g.error_code),
        })
    }

    pub(super) fn on_rst(&mut self, r: RstStreamFrame) -> Result<(), H2Error> {
        self.rst_flood.record(Instant::now())?;
        if self
            .streams
            .get(r.stream_id)
            .is_some_and(|actor| actor.state.is_reset_local() || actor.remote_done)
        {
            return Ok(());
        }
        if let Some(actor) = self.streams.get_mut(r.stream_id)
            && let Err(e) = actor
                .state
                .transition(StreamEvent::RecvRstStream(r.error_code))
        {
            tracing::debug!(error = ?e, stream_id = r.stream_id, "h2 RST_STREAM on stream in unexpected state");
        }
        let err = H2Error::Stream {
            stream_id: r.stream_id,
            code: r.error_code,
        };
        self.fail_stream(r.stream_id, err);
        Ok(())
    }

    pub(super) fn on_push(&mut self, pp: PushPromiseFrame) -> Result<(), H2Error> {
        if self.local_settings_acked && self.config.setting(SettingId::EnablePush) == Some(0) {
            return Err(H2Error::Connection {
                code: ErrorCode::ProtocolError,
                reason: "server pushed after acknowledging disabled push".into(),
            });
        }
        let decoded = self.decoder.decode_block(&pp.fragment, |_, _, _| {});
        self.writer.write_frame(&RstStreamFrame {
            stream_id: pp.promised_stream_id,
            error_code: ErrorCode::Cancel,
        })?;
        decoded.map_err(H2Error::Hpack)?;
        Ok(())
    }
}
