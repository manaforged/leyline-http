use std::io;
use std::net::SocketAddr;

use tokio::net::{TcpStream, UdpSocket};

use super::{addr_len, encode_addr, udp_associate};
use crate::tls::error::TlsError;

const HEADER_PREFIX: [u8; 3] = [0x00, 0x00, 0x00];

pub(crate) struct Socks5Udp {
    socket: UdpSocket,
    control: TcpStream,
    header: Vec<u8>,
}

impl Socks5Udp {
    pub(crate) async fn open<C: crate::tls::TlsHandshake>(
        connector: &C,
        proxy: &url::Url,
        host: &str,
        port: u16,
    ) -> Result<(Self, SocketAddr), TlsError> {
        let mut header = HEADER_PREFIX.to_vec();
        header.extend_from_slice(&encode_addr(host, port)?);
        let (control, relay) = udp_associate(connector, proxy).await?;
        let socket = UdpSocket::bind(crate::util::unspecified_for(relay))
            .await
            .map_err(TlsError::proxy_io)?;
        socket.connect(relay).await.map_err(TlsError::proxy_io)?;
        Ok((
            Self {
                socket,
                control,
                header,
            },
            relay,
        ))
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub(crate) async fn send(&self, datagram: &[u8]) -> io::Result<usize> {
        let mut packet = Vec::with_capacity(self.header.len() + datagram.len());
        packet.extend_from_slice(&self.header);
        packet.extend_from_slice(datagram);
        self.socket.send(&packet).await?;
        Ok(datagram.len())
    }

    pub(crate) async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        tokio::select! {
            received = self.recv_unwrapped(buf) => received,
            closed = self.control_closed() => Err(closed),
        }
    }

    async fn recv_unwrapped(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let len = self.socket.recv(buf).await?;
            if let Some(offset) = payload_offset(&buf[..len]) {
                buf.copy_within(offset..len, 0);
                return Ok(len - offset);
            }
        }
    }

    async fn control_closed(&self) -> io::Error {
        let mut probe = [0u8; 1];
        loop {
            if let Err(e) = self.control.readable().await {
                return e;
            }
            match self.control.try_read(&mut probe) {
                Ok(0) => {
                    return io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "socks5: UDP ASSOCIATE control connection closed",
                    );
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return e,
            }
        }
    }
}

fn payload_offset(packet: &[u8]) -> Option<usize> {
    let (&[0x00, 0x00, 0x00, atyp], rest) = packet.split_first_chunk::<4>()? else {
        return None;
    };
    let offset = 4 + addr_len(atyp, rest.first().copied()).ok()?;
    (offset <= packet.len()).then_some(offset)
}
