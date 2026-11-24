use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;

use super::*;
use crate::quic::transport::DatagramTransport;

const RECV_BATCH_LIMIT: usize = 64;
const CHANNEL_BATCH_LIMIT: usize = 64;
const TIMER_RESET_SLACK: Duration = Duration::from_millis(1);

pub(super) struct H3Loop {
    socket: DatagramTransport,
    conn: Box<quiche::Connection>,
    h3: quiche::h3::Connection,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    priority_update: bool,
    command_rx: mpsc::Receiver<H3Command>,
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    open_streams: Arc<AtomicUsize>,
    last_idle: Arc<crate::pool::types::Stamp>,
    streams: HashMap<u64, H3Stream>,
    egress: EgressBatch,
    buf: Vec<u8>,
    pending: VecDeque<H3Command>,
    admit_cap: Option<usize>,
    commands_closed: bool,
    draining: bool,
    sleep: Pin<Box<tokio::time::Sleep>>,
    sleep_deadline: Option<tokio::time::Instant>,
    last_cancel_sweep: Instant,
    sweep_due: bool,
}

type DatagramStop = (String, fn(String) -> H3SendError);

fn closed_stop(msg: String) -> DatagramStop {
    (msg, H3SendError::Closed)
}

impl H3Loop {
    pub(super) fn new(driver: H3Driver) -> Self {
        let EstablishedH3 {
            socket,
            conn,
            h3,
            peer_addr,
            local_addr,
            max_udp_payload,
            priority_update,
            tls: _,
        } = driver.established;
        Self {
            socket,
            conn,
            h3,
            peer_addr,
            local_addr,
            priority_update,
            command_rx: driver.command_rx,
            body_chunk_tx: driver.body_chunk_tx,
            body_chunk_rx: driver.body_chunk_rx,
            closed: driver.closed,
            open_streams: driver.open_streams,
            last_idle: driver.last_idle,
            streams: driver.streams,
            egress: EgressBatch::new(max_udp_payload),
            buf: vec![0u8; crate::quic::connection::UDP_RECV_BUFFER],
            pending: VecDeque::new(),
            admit_cap: None,
            commands_closed: false,
            draining: false,
            sleep: Box::pin(tokio::time::sleep(crate::quic::TIMER_FALLBACK)),
            sleep_deadline: None,
            last_cancel_sweep: Instant::now(),
            sweep_due: false,
        }
    }

    pub(super) async fn turn(&mut self) -> ControlFlow<()> {
        let backpressured = self.advance();
        let len = self.streams.len();
        let prev = self.open_streams.swap(len, Ordering::Relaxed);
        if prev > 0 && len == 0 {
            self.last_idle.touch(Instant::now());
        }
        self.settle().await?;
        self.wait(backpressured).await
    }

    fn advance(&mut self) -> bool {
        if self.sweep_due
            || !self.pending.is_empty()
            || self.last_cancel_sweep.elapsed() >= CANCEL_SWEEP_INTERVAL
        {
            sweep_cancelled_streams(
                &mut self.h3,
                &mut self.conn,
                &mut self.streams,
                &mut self.pending,
            );
            self.sweep_due = false;
            self.last_cancel_sweep = Instant::now();
        }
        start_pending(
            &mut self.h3,
            &mut self.conn,
            &mut self.streams,
            &mut self.pending,
            &self.body_chunk_tx,
            self.admit_cap,
            self.priority_update,
        );
        write_pending_request_bodies(&mut self.h3, &mut self.conn, &mut self.streams);
        pump_streaming_bodies(
            &mut self.h3,
            &mut self.conn,
            &mut self.streams,
            &mut self.buf,
        )
    }

    async fn settle(&mut self) -> ControlFlow<()> {
        if let Err(err) = self.flush().await {
            let kind = if matches!(err, crate::quic::error::QuicError::Tls(_)) {
                H3SendError::Tls
            } else {
                H3SendError::Closed
            };
            return self.stop(err.to_string(), kind);
        }
        if self.conn.is_closed() {
            let reason = close_reason("h3", 0, &self.conn);
            let kind = if crate::quic::connection::is_crypto_close(&self.conn) {
                H3SendError::Tls
            } else {
                H3SendError::Closed
            };
            return self.stop(reason, kind);
        }
        if self.commands_closed && self.streams.is_empty() && self.pending.is_empty() {
            self.closed.store(true, Ordering::Release);
            self.open_streams.store(0, Ordering::Relaxed);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn arm_sleep(&mut self, timeout: Duration) {
        let target = crate::util::saturating_after_tokio(tokio::time::Instant::now(), timeout);
        let reset = self.sleep_deadline.is_none_or(|current| {
            target < current || target.saturating_duration_since(current) >= TIMER_RESET_SLACK
        });
        if reset {
            self.sleep.as_mut().reset(target);
            self.sleep_deadline = Some(target);
        }
    }

    async fn wait(&mut self, backpressured: bool) -> ControlFlow<()> {
        let timeout = self.wake_timeout(backpressured);
        self.arm_sleep(timeout);
        tokio::select! {
            cmd = self.command_rx.recv(), if self.accepting_commands() => self.on_command_batch(cmd),
            chunk = self.body_chunk_rx.recv() => self.on_body_chunk_batch(chunk),
            recv = self.socket.recv(&mut self.buf) => {
                if let Err((reason, kind)) = self.on_datagram_batch(recv) {
                    return self.stop(reason, kind);
                }
            }
            _ = &mut self.sleep => {
                self.sleep_deadline = None;
                self.sweep_due = true;
                self.conn.on_timeout();
            }
        }
        ControlFlow::Continue(())
    }

    fn flush(&mut self) -> impl Future<Output = Result<(), crate::quic::error::QuicError>> {
        flush_egress(&self.socket, &mut self.conn, &mut self.egress)
    }

    fn stop(&mut self, reason: String, stream_err: fn(String) -> H3SendError) -> ControlFlow<()> {
        super::fail_all_with(
            &mut self.streams,
            &mut self.pending,
            &mut self.command_rx,
            &self.closed,
            reason,
            stream_err,
        );
        self.open_streams.store(0, Ordering::Relaxed);
        ControlFlow::Break(())
    }

    fn accepting_commands(&self) -> bool {
        !self.commands_closed && (self.draining || self.pending.is_empty())
    }

    fn wake_timeout(&self, backpressured: bool) -> Duration {
        let mut timeout = self.conn.timeout().unwrap_or(crate::quic::TIMER_FALLBACK);
        if backpressured {
            timeout = timeout.min(STREAM_PUMP_INTERVAL);
        }
        if !self.streams.is_empty() || !self.pending.is_empty() {
            timeout = timeout.min(CANCEL_SWEEP_INTERVAL);
        }
        timeout
    }

    fn on_command(&mut self, cmd: Option<H3Command>) {
        match cmd {
            Some(cmd) if self.draining => reject_unsent(cmd, GOAWAY_NOT_SENT.into()),
            Some(cmd) => self.pending.push_back(cmd),
            None => self.commands_closed = true,
        }
    }

    fn on_command_batch(&mut self, first: Option<H3Command>) {
        self.on_command(first);
        if self.commands_closed {
            return;
        }
        let extra = if self.draining {
            CHANNEL_BATCH_LIMIT - 1
        } else {
            let bidi =
                usize::try_from(self.conn.peer_streams_left_bidi()).unwrap_or(CHANNEL_BATCH_LIMIT);
            let admit = self.admit_cap.map_or(CHANNEL_BATCH_LIMIT, |cap| {
                cap.saturating_sub(self.streams.len())
            });
            bidi.min(admit)
                .min(CHANNEL_BATCH_LIMIT)
                .saturating_sub(self.pending.len())
        };
        for _ in 0..extra {
            match self.command_rx.try_recv() {
                Ok(cmd) => self.on_command(Some(cmd)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.commands_closed = true;
                    break;
                }
            }
        }
    }

    fn on_body_chunk(&mut self, chunk: Option<H3BodyChunk>) {
        if let Some(chunk) = chunk {
            on_request_body_chunk(&mut self.h3, &mut self.conn, &mut self.streams, chunk);
        }
    }

    fn on_body_chunk_batch(&mut self, first: Option<H3BodyChunk>) {
        self.on_body_chunk(first);
        for _ in 1..CHANNEL_BATCH_LIMIT {
            match self.body_chunk_rx.try_recv() {
                Ok(chunk) => self.on_body_chunk(Some(chunk)),
                Err(_) => break,
            }
        }
    }

    fn ingest_datagram(&mut self, len: usize) -> Result<(), DatagramStop> {
        if len == 0 {
            return Ok(());
        }
        let recv_info = quiche::RecvInfo {
            from: self.peer_addr,
            to: self.local_addr,
        };
        if let Err(e) = self.conn.recv(&mut self.buf[..len], recv_info) {
            if matches!(e, quiche::Error::TlsFail)
                || crate::quic::connection::is_crypto_close(&self.conn)
            {
                let stack = leyline_bssl::error::ErrorStack::get();
                let reason = if stack.errors().is_empty() {
                    format!("quic recv: {e}")
                } else {
                    format!("quic recv: {e}: {stack}")
                };
                return Err((reason, H3SendError::Tls));
            }
            return Err(closed_stop(format!("quic recv: {e}")));
        }
        Ok(())
    }

    fn drain_h3(&mut self) -> Result<(), DatagramStop> {
        let goaway = Drain {
            h3: &mut self.h3,
            conn: &mut self.conn,
            streams: &mut self.streams,
            scratch: &mut self.buf,
            admit: &mut self.admit_cap,
            draining: self.draining,
        }
        .run()
        .map_err(closed_stop)?;
        if goaway {
            self.begin_draining();
        }
        Ok(())
    }

    fn on_datagram_batch(&mut self, first: io::Result<usize>) -> Result<(), DatagramStop> {
        let len = first.map_err(|e| closed_stop(format!("udp recv: {e}")))?;
        self.ingest_datagram(len)?;
        self.drain_h3()?;
        for _ in 1..RECV_BATCH_LIMIT {
            if self.conn.is_closed() {
                break;
            }
            match self.socket.try_recv(&mut self.buf) {
                Ok(n) => {
                    self.ingest_datagram(n)?;
                    self.drain_h3()?;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(closed_stop(format!("udp recv: {e}"))),
            }
        }
        Ok(())
    }

    fn begin_draining(&mut self) {
        self.draining = true;
        self.closed.store(true, Ordering::Release);
        for cmd in self.pending.drain(..) {
            reject_unsent(cmd, GOAWAY_NOT_SENT.into());
        }
        tracing::debug!(
            target: "leyline::quic",
            "h3 server GOAWAY: connection draining, pool handle closed"
        );
    }
}
