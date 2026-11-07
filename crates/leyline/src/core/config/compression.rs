use crate::core::request::Encoding;

pub(crate) const DEFAULT_MAX_BODY_SIZE: usize = 100 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_HEADER_LIST_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BodyRules {
    pub(crate) gzip: bool,
    pub(crate) brotli: bool,
    pub(crate) deflate: bool,
    pub(crate) zstd: bool,
    pub(crate) decode: bool,
    pub(crate) max_body_size: usize,
    pub(crate) max_error_body: usize,
}

impl Default for BodyRules {
    fn default() -> Self {
        Self {
            gzip: true,
            brotli: true,
            deflate: true,
            zstd: true,
            decode: true,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            max_error_body: 64 * 1024,
        }
    }
}

impl BodyRules {
    pub(crate) fn offer(&mut self, codings: impl IntoIterator<Item = Encoding>) {
        self.gzip = false;
        self.brotli = false;
        self.deflate = false;
        self.zstd = false;
        for coding in codings {
            match coding {
                Encoding::Gzip => self.gzip = true,
                Encoding::Brotli => self.brotli = true,
                Encoding::Deflate => self.deflate = true,
                Encoding::Zstd => self.zstd = true,
            }
        }
    }

    pub(crate) fn accept_encoding(&self) -> Option<&'static str> {
        let mask = usize::from(self.gzip && cfg!(feature = "compression-gzip"))
            | usize::from(self.deflate && cfg!(feature = "compression-deflate")) << 1
            | usize::from(self.brotli && cfg!(feature = "compression-brotli")) << 2
            | usize::from(self.zstd && cfg!(feature = "compression-zstd")) << 3;
        (mask != 0).then(|| CODEC_LISTS[mask])
    }

    pub(crate) fn allows(&self, encoding: &str) -> bool {
        match encoding {
            "identity" | "" => true,
            _ if !self.decode => false,
            "gzip" | "x-gzip" => self.gzip,
            "br" => self.brotli,
            "deflate" => self.deflate,
            "zstd" => self.zstd,
            _ => false,
        }
    }
}

const CODEC_LISTS: [&str; 16] = [
    "",
    "gzip",
    "deflate",
    "gzip, deflate",
    "br",
    "gzip, br",
    "deflate, br",
    "gzip, deflate, br",
    "zstd",
    "gzip, zstd",
    "deflate, zstd",
    "gzip, deflate, zstd",
    "br, zstd",
    "gzip, br, zstd",
    "deflate, br, zstd",
    "gzip, deflate, br, zstd",
];
