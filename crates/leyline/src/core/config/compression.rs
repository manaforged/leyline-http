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

    pub(crate) fn accept_encoding(&self) -> Option<String> {
        let codecs = [
            ("gzip", self.gzip && cfg!(feature = "compression-gzip")),
            (
                "deflate",
                self.deflate && cfg!(feature = "compression-deflate"),
            ),
            ("br", self.brotli && cfg!(feature = "compression-brotli")),
            ("zstd", self.zstd && cfg!(feature = "compression-zstd")),
        ];
        let enabled: Vec<&str> = codecs
            .iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| *name)
            .collect();
        (!enabled.is_empty()).then(|| enabled.join(", "))
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
