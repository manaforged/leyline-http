use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const MAX_TTL: u32 = u8::MAX as u32;
const MAX_MSS: u32 = u16::MAX as u32;
const MAX_WINDOW_SIZE: u32 = u16::MAX as u32;
const MAX_WINDOW_SCALE: u32 = 14;

fn validate_tcp_bounds(
    ttl: u32,
    mss: u32,
    window_size: u32,
    window_scale: u32,
) -> std::result::Result<(), String> {
    if ttl > MAX_TTL {
        return Err(format!("ttl {ttl} exceeds {MAX_TTL}"));
    }
    if mss > MAX_MSS {
        return Err(format!("mss {mss} exceeds {MAX_MSS}"));
    }
    if window_size > MAX_WINDOW_SIZE {
        return Err(format!(
            "window_size {window_size} exceeds {MAX_WINDOW_SIZE}"
        ));
    }
    if window_scale > MAX_WINDOW_SCALE {
        return Err(format!(
            "window_scale {window_scale} exceeds {MAX_WINDOW_SCALE}"
        ));
    }
    Ok(())
}

/// TCP socket settings applied to each connection to match a client's network fingerprint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTcpProfile")]
#[non_exhaustive]
pub struct TcpProfile {
    pub(crate) ttl: u32,
    pub(crate) mss: u32,
    pub(crate) window_size: u32,
    pub(crate) df: bool,
    pub(crate) window_scale: u32,
    pub(crate) no_delay: bool,
    pub(crate) options: Vec<u8>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawTcpProfile {
    ttl: u32,
    mss: u32,
    window_size: u32,
    df: bool,
    window_scale: u32,
    no_delay: bool,
    options: Vec<u8>,
}

impl TryFrom<RawTcpProfile> for TcpProfile {
    type Error = String;

    fn try_from(raw: RawTcpProfile) -> std::result::Result<Self, Self::Error> {
        validate_tcp_bounds(raw.ttl, raw.mss, raw.window_size, raw.window_scale)?;
        Ok(Self {
            ttl: raw.ttl,
            mss: raw.mss,
            window_size: raw.window_size,
            df: raw.df,
            window_scale: raw.window_scale,
            no_delay: raw.no_delay,
            options: raw.options,
        })
    }
}

impl TcpProfile {
    /// Creates an empty TCP profile that leaves every socket option at the operating system default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a JA4T string (`window_options_mss_scale`) into a TCP profile.
    pub fn from_ja4t(raw: impl AsRef<str>) -> Result<Self> {
        let ja4t = Ja4t::parse(raw.as_ref())?;
        let mut tcp = Self::new();
        ja4t.apply(&mut tcp);
        Ok(tcp)
    }

    /// Returns the IPv4 TTL or IPv6 hop limit; 0 leaves the OS default.
    #[must_use]
    pub fn ttl(&self) -> u32 {
        self.ttl
    }

    /// Returns the TCP maximum segment size in bytes.
    #[must_use]
    pub fn mss(&self) -> u32 {
        self.mss
    }

    /// Returns the TCP receive window size in bytes.
    #[must_use]
    pub fn window_size(&self) -> u32 {
        self.window_size
    }

    /// Returns whether the IP Don't Fragment bit is set.
    #[must_use]
    pub fn df(&self) -> bool {
        self.df
    }

    /// Returns the TCP window scale factor.
    #[must_use]
    pub fn window_scale(&self) -> u32 {
        self.window_scale
    }

    /// Returns whether `TCP_NODELAY` is set on the socket.
    #[must_use]
    pub fn no_delay(&self) -> bool {
        self.no_delay
    }

    /// Returns the TCP option kinds in the order the client sends them.
    #[must_use]
    pub fn options(&self) -> &[u8] {
        &self.options
    }

    /// Returns the JA4T fingerprint string (`window_options_mss_scale`) of this TCP profile.
    #[must_use]
    pub fn ja4t(&self) -> String {
        crate::audit::compute_ja4t(self)
    }
}

impl FromStr for TcpProfile {
    type Err = Error;

    fn from_str(toml: &str) -> Result<Self> {
        toml::from_str::<Self>(toml).map_err(|err| Error::config(format!("TCP profile: {err}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ja4t {
    window_size: u32,
    options: Vec<u8>,
    mss: u32,
    window_scale: u32,
}

impl Ja4t {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let mut parts = raw.trim().split('_');
        let (Some(window), Some(options), Some(mss), Some(scale), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(ja4t_error("expected 4 underscore-separated fields"));
        };
        let window_size = parse_ja4t_u32("window_size", window)?;
        let options = parse_ja4t_options(options)?;
        let mss = parse_ja4t_u32("mss", mss)?;
        let window_scale = parse_ja4t_u32("window_scale", scale)?;
        validate_tcp_bounds(0, mss, window_size, window_scale).map_err(ja4t_error)?;
        Ok(Self {
            window_size,
            options,
            mss,
            window_scale,
        })
    }

    pub(crate) fn apply(&self, tcp: &mut TcpProfile) {
        tcp.window_size = self.window_size;
        tcp.options.clone_from(&self.options);
        tcp.mss = self.mss;
        tcp.window_scale = self.window_scale;
    }
}

fn parse_ja4t_u32(field: &str, text: &str) -> Result<u32> {
    text.parse::<u32>()
        .map_err(|err| ja4t_error(format!("invalid {field} `{text}`: {err}")).with_source(err))
}

fn parse_ja4t_options(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    text.split('-')
        .map(|item| {
            item.parse::<u8>().map_err(|err| {
                ja4t_error(format!("invalid TCP option `{item}`: {err}")).with_source(err)
            })
        })
        .collect()
}

fn ja4t_error(why: impl std::fmt::Display) -> Error {
    Error::config(format!("JA4T fingerprint: {why}"))
}
