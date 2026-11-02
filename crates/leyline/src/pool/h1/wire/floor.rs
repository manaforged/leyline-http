use super::*;
use std::io::{Read, Write};

use leyline_bssl::ssl::{Ssl, SslStream, SslVerifyMode};

use crate::header_str::HeaderStr;
use crate::pool::h1::parse::parse_head;

/// The least work an HTTP/1.1 GET over TLS can do with leyline's own encoder, parser and BoringSSL: the floor the full client is measured against.
pub struct Floor {
    out: Vec<u8>,
    rx: Vec<u8>,
    headers: Vec<HeaderPair>,
}

/// Opens a blocking TLS connection to `addr` that offers only HTTP/1.1.
pub fn floor_connect(addr: &str) -> io::Result<SslStream<std::net::TcpStream>> {
    let tcp = std::net::TcpStream::connect(addr)?;
    tcp.set_nodelay(true)?;
    let mut context = crate::profile::BrowserProfile::bare_shared()
        .build_ssl_context(crate::TlsMinVersion::Tls12)
        .map_err(|e| io::Error::other(e.to_string()))?;
    context.set_verify(SslVerifyMode::NONE);
    context
        .set_alpn_protos(crate::tls::alpn::HTTP11_WIRE)
        .map_err(io::Error::other)?;
    let ssl = Ssl::new(&context.build()).map_err(io::Error::other)?;
    ssl.connect(tcp)
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

    /// Sends a GET for `url` on `stream` and returns the status and body of a content-length response.
    pub fn get<S: Read + Write>(
        &mut self,
        stream: &mut S,
        url: &url::Url,
    ) -> Result<(u16, Bytes), Error> {
        let (target, authority) = head::target(url, H1Target::OriginForm)?;
        let headers = &self.headers;
        head::Head {
            method: "GET",
            target: &target,
            host: (!contains_header(headers, "host")).then_some(&*authority),
            headers,
            length: None,
            keep_alive: !contains_header(headers, "connection"),
        }
        .encode(&mut self.out);
        stream.write_all(&self.out)?;
        let mut filled = 0;
        let end = loop {
            if let Some(end) = memchr::memmem::find(&self.rx[..filled], b"\r\n\r\n") {
                break end;
            }
            filled += read_some(stream, &mut self.rx[filled..])?;
        };
        let head = Bytes::copy_from_slice(&self.rx[..end]);
        let (status, fields, _, _) = parse_head(&head)?;
        let len: usize = fields
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse().ok())
            .ok_or_else(|| Error::from(io::Error::other("no content-length")))?;
        let start = end + 4;
        while filled < start + len {
            filled += read_some(stream, &mut self.rx[filled..])?;
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
