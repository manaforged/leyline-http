use std::io;
use std::net::SocketAddr;
use std::ops::ControlFlow;

use super::*;
use crate::quic::transport::DatagramTransport;

const GOAWAY_NOT_SENT: &str = "server sent GOAWAY: request not sent";

pub(super) struct H3Loop {
    socket: DatagramTransport,
    conn: Box<quiche::Connection>,
    h3: quiche::h3::Connection,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    priority_update: bool,
    max_response_body_bytes: u64,
    command_rx: mpsc::Receiver<H3Command>,
    body_chunk_tx: mpsc::Sender<H3BodyChunk>,
    body_chunk_rx: mpsc::Receiver<H3BodyChunk>,
    closed: Arc<AtomicBool>,
    streams: HashMap<u64, H3Stream>,
    out: Vec<u8>,
    buf: Vec<u8>,
    pending: VecDeque<H3Command>,
    admit_cap: Option<usize>,
    commands_closed: bool,
    draining: bool,
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
            max_response_body_bytes,
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
            max_response_body_bytes,
            command_rx: driver.command_rx,
            body_chunk_tx: driver.body_chunk_tx,
            body_chunk_rx: driver.body_chunk_rx,
            closed: driver.closed,
            streams: driver.streams,
            out: vec![0u8; max_udp_payload],
            buf: vec![0u8; 65_535],
            pending: VecDeque::new(),
            admit_cap: None,
            commands_closed: false,
            draining: false,
        }
    }

    pub(super) async fn turn(&mut self) -> ControlFlow<()> {
        let backpressured = self.advance();
        self.settle().await?;
        self.wait(backpressured).await
    }

    fn advance(&mut self) -> bool {
        sweep_cancelled_streams(&mut self.h3, &mut self.conn, &mut self.streams);
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
        if let Err(reason) = self.flush().await {
            return self.stop(reason);
        }
        if self.conn.is_closed() {
            let reason = close_reason("h3", 0, &self.conn);
            return self.stop(reason);
        }
        if self.is_idle() {
            self.closed.store(true, Ordering::Release);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    async fn wait(&mut self, backpressured: bool) -> ControlFlow<()> {
        let timeout = self.wake_timeout(backpressured);
        tokio::select! {
            cmd = self.command_rx.recv(), if !self.commands_closed => self.on_command(cmd),
            chunk = self.body_chunk_rx.recv() => self.on_body_chunk(chunk),
            recv = self.socket.recv(&mut self.buf) => {
                if let Err(reason) = self.on_datagram(recv) {
                    return self.stop(reason);
                }
            }
            _ = tokio::time::sleep(timeout) => self.conn.on_timeout(),
        }
        ControlFlow::Continue(())
    }

    async fn flush(&mut self) -> Result<(), String> {
        flush_egress(&self.socket, &mut self.conn, &mut self.out).await
    }

    fn stop(&mut self, reason: String) -> ControlFlow<()> {
        fail_all(
            &mut self.streams,
            &mut self.pending,
            &mut self.command_rx,
            &self.closed,
            reason,
        );
        ControlFlow::Break(())
    }

    fn is_idle(&self) -> bool {
        self.commands_closed && self.streams.is_empty() && self.pending.is_empty()
    }

    fn wake_timeout(&self, backpressured: bool) -> Duration {
        let mut timeout = self.conn.timeout().unwrap_or(Duration::from_secs(5));
        if backpressured {
            timeout = timeout.min(STREAM_PUMP_INTERVAL);
        }
        if !self.streams.is_empty() {
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

    fn on_body_chunk(&mut self, chunk: Option<H3BodyChunk>) {
        if let Some(chunk) = chunk {
            on_request_body_chunk(&mut self.h3, &mut self.conn, &mut self.streams, chunk);
        }
    }

    fn on_datagram(&mut self, recv: io::Result<usize>) -> Result<(), String> {
        let len = recv.map_err(|e| format!("udp recv: {e}"))?;
        let recv_info = quiche::RecvInfo {
            from: self.peer_addr,
            to: self.local_addr,
        };
        self.conn
            .recv(&mut self.buf[..len], recv_info)
            .map_err(|e| format!("quic recv: {e}"))?;
        let goaway = drain_h3_events(
            &mut self.h3,
            &mut self.conn,
            &mut self.streams,
            &mut self.pending,
            &mut self.buf,
            self.max_response_body_bytes,
            &mut self.admit_cap,
        )?;
        if goaway {
            self.begin_draining();
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
