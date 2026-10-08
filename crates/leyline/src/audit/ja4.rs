use crate::audit::{hash12, non_grease_cipher_ids, non_grease_ext_ids};
use crate::iana::sigalg_id;

#[derive(Debug)]
pub struct Ja4Input<'a> {
    pub ciphers: &'a [String],
    pub sigalgs: &'a [String],
    pub curves: &'a [String],
    pub extension_ids: &'a [u16],
    pub tls_version: &'a str,
    pub has_sni: bool,
    pub alpn: &'a str,
}

pub fn compute_ja4(input: &Ja4Input<'_>) -> String {
    let section_a = input.compute_section_a();
    let section_b = input.compute_section_b();
    let section_c = input.compute_section_c();
    format!("{section_a}_{section_b}_{section_c}")
}

impl Ja4Input<'_> {
    fn compute_section_a(&self) -> String {
        let proto = "t";
        let version = match self.tls_version {
            "1.3" => "13",
            "1.2" => "12",
            "1.1" => "11",
            "1.0" => "10",
            _ => "00",
        };

        let sni = if self.has_sni { "d" } else { "i" };

        let cipher_count = non_grease_cipher_ids(self.ciphers).len().min(99);

        let ext_count = non_grease_ext_ids(self.extension_ids).len().min(99);

        let alpn = if self.alpn.is_empty() {
            "00".to_string()
        } else {
            let bytes = self.alpn.as_bytes();
            let first = bytes[0] as char;
            let last = bytes[bytes.len() - 1] as char;
            format!("{first}{last}")
        };

        format!("{proto}{version}{sni}{cipher_count:02}{ext_count:02}{alpn}")
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
            .filter(|id| *id != 0x0000 && *id != 0x0010)
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
