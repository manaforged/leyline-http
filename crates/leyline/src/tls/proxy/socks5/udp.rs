use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::{TcpStream, UdpSocket};

use super::{CMD_UDP_ASSOCIATE, addr_len, encode_addr, encode_socket_addr, open_control, request};
use crate::tls::error::TlsError;

const HEADER_PREFIX: [u8; 3] = [0x00, 0x00, 0x00];

async fn udp_associate<C: crate::tls::TlsHandshake>(
    connector: &C,
    proxy: &url::Url,
) -> Result<(TcpStream, SocketAddr), TlsError> {
    let mut control = open_control(connector, proxy).await?;
    let unspecified = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    let (atyp, body) = request(
        &mut control,
        CMD_UDP_ASSOCIATE,
        &encode_socket_addr(unspecified),
    )
    .await?;
    let relay = match decode_bound(atyp, &body) {
        Bound::Ip(addr) if addr.ip().is_unspecified() => {
            let proxy_ip = control.peer_addr().map_err(TlsError::proxy_io)?.ip();
            SocketAddr::new(proxy_ip, addr.port())
        }
        Bound::Ip(addr) => addr,
        Bound::Domain => {
            return Err(TlsError::proxy(
                "socks5: UDP ASSOCIATE relay address is a domain name",
            ));
        }
    };
    Ok((control, relay))
}

enum Bound {
    Ip(SocketAddr),
    Domain,
}

fn decode_bound(atyp: u8, body: &[u8]) -> Bound {
    let port_at = body.len() - 2;
    let port = u16::from_be_bytes([body[port_at], body[port_at + 1]]);
    let ip = match atyp {
        0x01 => <[u8; 4]>::try_from(&body[..4]).map(IpAddr::from).ok(),
        0x04 => <[u8; 16]>::try_from(&body[..16]).map(IpAddr::from).ok(),
        _ => None,
    };
    ip.map_or(Bound::Domain, |ip| Bound::Ip(SocketAddr::new(ip, port)))
}

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
