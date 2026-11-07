use super::*;
use std::io::{Read, Write};

use leyline_bssl::ssl::{Ssl, SslStream, SslVerifyMode};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::header_str::HeaderStr;
use crate::pool::h1::parse::parse_head;

/// The least work an HTTP/1.1 GET over TLS can do with leyline's own encoder, parser and BoringSSL: the floor the full client is measured against.
pub struct Floor {
    out: Vec<u8>,
    rx: Vec<u8>,
    headers: Vec<HeaderPair>,
}

fn floor_ssl() -> io::Result<Ssl> {
    let mut context = crate::profile::BrowserProfile::bare_shared()
        .build_ssl_context(crate::TlsMinVersion::Tls12)
        .map_err(|e| io::Error::other(e.to_string()))?;
    context.set_verify(SslVerifyMode::NONE);
    context
        .set_alpn_protos(crate::tls::alpn::HTTP11_WIRE)
        .map_err(io::Error::other)?;
    Ssl::new(&context.build()).map_err(io::Error::other)
}

/// Opens a blocking TLS connection to `addr` that offers only HTTP/1.1.
pub fn floor_connect(addr: &str) -> io::Result<SslStream<std::net::TcpStream>> {
    let tcp = std::net::TcpStream::connect(addr)?;
    tcp.set_nodelay(true)?;
    floor_ssl()?
        .connect(tcp)
        .map_err(|e| io::Error::other(format!("{e:?}")))
}

/// Opens a Tokio TLS connection to `addr` that offers only HTTP/1.1.
pub async fn floor_connect_async(
    addr: &str,
) -> io::Result<leyline_bssl_tokio::SslStream<tokio::net::TcpStream>> {
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    tcp.set_nodelay(true)?;
    leyline_bssl_tokio::SslStreamBuilder::new(floor_ssl()?, tcp)
        .connect()
        .await
        .map_err(|e| io::Error::other(format!("{e:?}")))
}

impl Floor {
    /// Prepares requests that send `headers` after the request line.
    pub fn new(headers: &[(String, String)]) -> Self {
        Self {
            out: Vec::new(),
            rx: vec![0; 64 * 1024],
            headers: headers
                .iter()
                .map(|(name, value)| (Cow::Owned(name.clone()), HeaderStr::from(value.as_str())))
                .collect(),
        }
    }

    fn encode(&mut self, url: &url::Url) -> Result<(), Error> {
        let (target, authority) = head::target(url, H1Target::OriginForm)?;
        let headers = &self.headers;
        head::Head {
            method: "GET",
            target: &target,
            host: (!contains_header(headers, "host")).then_some(&*authority),
            headers,
            lines: None,
            length: None,
            keep_alive: !contains_header(headers, "connection"),
        }
        .encode(&mut self.out);
        Ok(())
    }

    fn head_end(&self, filled: usize) -> Option<usize> {
        memchr::memmem::find(&self.rx[..filled], b"\r\n\r\n")
    }

    fn parse(&self, end: usize) -> Result<(u16, usize, usize), Error> {
        let head = Bytes::copy_from_slice(&self.rx[..end]);
        let (status, fields, _) = parse_head(&head)?;
        let len: usize = fields
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse().ok())
            .ok_or_else(|| Error::from(io::Error::other("no content-length")))?;
        Ok((status, end + 4, len))
    }

    /// Sends a GET for `url` on `stream` and returns the status and body of a content-length response.
    pub fn get<S: Read + Write>(
        &mut self,
        stream: &mut S,
        url: &url::Url,
    ) -> Result<(u16, Bytes), Error> {
        self.encode(url)?;
        stream.write_all(&self.out)?;
        let mut filled = 0;
        let end = loop {
            if let Some(end) = self.head_end(filled) {
                break end;
            }
            filled += read_some(stream, &mut self.rx[filled..])?;
        };
        let (status, start, len) = self.parse(end)?;
        while filled < start + len {
            filled += read_some(stream, &mut self.rx[filled..])?;
        }
        Ok((status, Bytes::copy_from_slice(&self.rx[start..start + len])))
    }

    /// Sends a GET for `url` on an async `stream` and returns the status and body of a content-length response.
    pub async fn get_async<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        stream: &mut S,
        url: &url::Url,
    ) -> Result<(u16, Bytes), Error> {
        self.encode(url)?;
        stream.write_all(&self.out).await?;
        let mut filled = 0;
        let end = loop {
            if let Some(end) = self.head_end(filled) {
                break end;
            }
            filled += read_some_async(stream, &mut self.rx[filled..]).await?;
        };
        let (status, start, len) = self.parse(end)?;
        while filled < start + len {
            filled += read_some_async(stream, &mut self.rx[filled..]).await?;
        }
        Ok((status, Bytes::copy_from_slice(&self.rx[start..start + len])))
    }
}

fn read_some<S: Read>(stream: &mut S, into: &mut [u8]) -> io::Result<usize> {
    match stream.read(into)? {
        0 => Err(io::ErrorKind::UnexpectedEof.into()),
        n => Ok(n),
    }
}

async fn read_some_async<S: AsyncRead + Unpin>(
    stream: &mut S,
    into: &mut [u8],
) -> io::Result<usize> {
    match stream.read(into).await? {
        0 => Err(io::ErrorKind::UnexpectedEof.into()),
        n => Ok(n),
    }
}
