pub(crate) const DEFAULT_MAX_BODY_SIZE: usize = 100 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_HEADER_LIST_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompressionConfig {
    pub(crate) gzip: bool,
    pub(crate) brotli: bool,
    pub(crate) deflate: bool,
    pub(crate) zstd: bool,
    pub(crate) max_body_size: usize,
    pub(crate) max_error_body: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            gzip: true,
            brotli: true,
            deflate: true,
            zstd: true,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            max_error_body: 64 * 1024,
        }
    }
}

impl CompressionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn none() -> Self {
        Self {
            gzip: false,
            brotli: false,
            deflate: false,
            zstd: false,
            ..Self::default()
        }
    }

    pub fn gzip(mut self, on: bool) -> Self {
        self.gzip = on;
        self
    }

    pub fn brotli(mut self, on: bool) -> Self {
        self.brotli = on;
        self
    }

    pub fn deflate(mut self, on: bool) -> Self {
        self.deflate = on;
        self
    }

    pub fn zstd(mut self, on: bool) -> Self {
        self.zstd = on;
        self
    }

    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }

    pub fn max_error_body(mut self, bytes: usize) -> Self {
        self.max_error_body = bytes;
        self
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
            "gzip" | "x-gzip" => self.gzip,
            "br" => self.brotli,
            "deflate" => self.deflate,
            "zstd" => self.zstd,
            "identity" | "" => true,
            _ => false,
        }
    }
}
