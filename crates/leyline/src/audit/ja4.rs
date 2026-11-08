use crate::audit::{hash12, non_grease_cipher_ids, non_grease_ext_ids};
use crate::iana::{ALPN, SERVER_NAME, sigalg_id};
use crate::profile::JA4_TCP as TCP;

const QUIC: char = 'q';
const TLS13: &str = "13";

/// The ClientHello a JA4 describes: its transport and the first ALPN protocol it offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientHello {
    /// A TCP ClientHello that offers `h2` first.
    TcpH2,
    /// A TCP ClientHello that offers only `http/1.1`.
    TcpHttp11,
    /// A QUIC ClientHello that offers `h3`.
    QuicH3,
}

impl ClientHello {
    fn transport(self) -> char {
        match self {
            Self::TcpH2 | Self::TcpHttp11 => TCP,
            Self::QuicH3 => QUIC,
        }
    }

    fn alpn(self) -> &'static str {
        match self {
            Self::TcpH2 => "h2",
            Self::TcpHttp11 => "h1",
            Self::QuicH3 => "h3",
        }
    }

    pub(crate) fn carrier(self) -> crate::profile::Carrier {
        match self {
            Self::TcpH2 | Self::TcpHttp11 => crate::profile::Carrier::Tcp,
            Self::QuicH3 => crate::profile::Carrier::Quic,
        }
    }
}

#[derive(Debug)]
pub struct Ja4Input<'a> {
    pub ciphers: &'a [String],
    pub sigalgs: &'a [String],
    pub extension_ids: &'a [u16],
    pub has_sni: bool,
    pub hello: ClientHello,
}

pub fn compute_ja4(input: &Ja4Input<'_>) -> String {
    let section_a = input.compute_section_a();
    let section_b = input.compute_section_b();
    let section_c = input.compute_section_c();
    format!("{section_a}_{section_b}_{section_c}")
}

impl Ja4Input<'_> {
    fn compute_section_a(&self) -> String {
        let proto = self.hello.transport();

        let sni = if self.has_sni { "d" } else { "i" };

        let cipher_count = non_grease_cipher_ids(self.ciphers).len().min(99);

        let ext_count = non_grease_ext_ids(self.extension_ids).len().min(99);

        let alpn = self.hello.alpn();

        format!("{proto}{TLS13}{sni}{cipher_count:02}{ext_count:02}{alpn}")
    }

    fn compute_section_b(&self) -> String {
        let mut ids = non_grease_cipher_ids(self.ciphers);

        ids.sort();

        let s: String = ids
            .iter()
            .map(|id| format!("{id:04x}"))
            .collect::<Vec<_>>()
            .join(",");

        hash12(&s)
    }

    fn compute_section_c(&self) -> String {
        let mut ext_ids: Vec<u16> = non_grease_ext_ids(self.extension_ids)
            .into_iter()
            .filter(|id| *id != SERVER_NAME && *id != ALPN)
            .collect();
        ext_ids.sort();

        let ext_str: String = ext_ids
            .iter()
            .map(|id| format!("{id:04x}"))
            .collect::<Vec<_>>()
            .join(",");

        let sigalg_str: String = self
            .sigalgs
            .iter()
            .filter_map(|s| sigalg_id(s))
            .map(|id| format!("{id:04x}"))
            .collect::<Vec<_>>()
            .join(",");

        let combined = format!("{ext_str}_{sigalg_str}");
        hash12(&combined)
    }
}

#[cfg(test)]
mod tests;
