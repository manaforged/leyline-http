use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;

use crate::h2::error::H2Error;
use crate::h2::frame::Frame;

use super::*;

enum Event {
    Written(Result<(), H2Error>),
    Read(Result<Option<Frame>, H2Error>),
    Command(Option<DriverCommand>),
    BodyChunk(BodyChunkIn),
    Ping(oneshot::Sender<()>),
    Sweep,
    FlushStalled,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Driver<T> {
    pub(super) async fn wait_event(
        &mut self,
        sweep_tick: &mut tokio::time::Interval,
        flush_tick: &mut tokio::time::Interval,
    ) -> Result<Option<bool>, H2Error> {
        let event = self.next_event(sweep_tick, flush_tick).await;
        self.on_event(event).await
    }

    async fn next_event(
        &mut self,
        sweep_tick: &mut tokio::time::Interval,
        flush_tick: &mut tokio::time::Interval,
    ) -> Event {
        let writing = self.writer.has_output();
        let reading = !self.writer.saturated();
        let accepting = self.accepting_commands();
        let taking_body = !self.writer.congested();
        let stalled = self.has_stalled();
        tokio::select! {
            biased;
            _ = sweep_tick.tick() => Event::Sweep,
            written = self.writer.write_some(), if writing => Event::Written(written),
            frame = self.reader.next(), if reading => Event::Read(frame),
            cmd = self.command_rx.recv(), if accepting => Event::Command(cmd),
            Some(chunk) = self.body_chunk_rx.recv(), if taking_body => Event::BodyChunk(chunk),
            Some(ack_tx) = self.ping_rx.recv() => Event::Ping(ack_tx),
            _ = flush_tick.tick(), if stalled => Event::FlushStalled,
        }
    }

    async fn on_event(&mut self, event: Event) -> Result<Option<bool>, H2Error> {
        match event {
            Event::Written(written) => self.output.wrote(written)?,
            Event::Read(frame) => {
                return Ok(self.on_read(frame).await?.is_continue().then_some(false));
            }
            Event::Command(cmd) => self.on_command_received(cmd).await?,
            Event::BodyChunk(chunk) => self.on_body_chunk(chunk).await?,
            Event::Ping(ack_tx) => self.on_command(DriverCommand::Ping { ack_tx }).await?,
            Event::Sweep => return Ok(Some(true)),
            Event::FlushStalled => self.flush_stalled().await?,
        }
        Ok(Some(false))
    }
}
