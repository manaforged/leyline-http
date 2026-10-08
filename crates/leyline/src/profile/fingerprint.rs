mod akamai;
mod ja3;
mod ja4;

use crate::iana::is_grease;
use crate::profile::{BrowserProfile, TlsProfile};
use crate::{Error, Kind};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FingerprintSpec {
    ja3: Option<String>,
    ja4_r: Option<String>,
    akamai: Option<String>,
    header_order: Option<Vec<String>>,
    base: Option<BrowserProfile>,
    name: Option<String>,
}

impl FingerprintSpec {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn ja3(mut self, raw: impl Into<String>) -> Self {
        self.ja3 = Some(raw.into());
        self
    }

    #[must_use]
    pub fn ja4_r(mut self, raw: impl Into<String>) -> Self {
        self.ja4_r = Some(raw.into());
        self
    }

    #[must_use]
    pub fn akamai(mut self, raw: impl Into<String>) -> Self {
        self.akamai = Some(raw.into());
        self
    }

    #[must_use]
    pub fn header_order<I, S>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.header_order = Some(order.into_iter().map(Into::into).collect());
        self
    }

    #[must_use]
    pub fn base(mut self, profile: BrowserProfile) -> Self {
        self.base = Some(profile);
        self
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

fn config_error(format: &str, why: impl std::fmt::Display) -> Error {
    Error::new(Kind::Config).with_message(format!("{format} fingerprint: {why}"))
}

impl BrowserProfile {
    pub fn from_fingerprint(spec: FingerprintSpec) -> crate::Result<Self> {
        let mut profile = spec.base.unwrap_or_else(|| (*Self::bare_shared()).clone());
        if let Some(name) = spec.name {
            profile.meta.name = name;
        }
        if spec.ja4_r.is_some() || spec.ja3.is_some() {
            profile.tls.fingerprint = None;
        }
        if let Some(raw) = spec.ja4_r.as_deref() {
            profile
                .tls
                .apply_ja4(raw)
                .map_err(|why| config_error("JA4_r", why))?;
        }
        if let Some(raw) = spec.ja3.as_deref() {
            profile
                .tls
                .apply_ja3(raw)
                .map_err(|why| config_error("JA3", why))?;
        }
        profile
            .tls
            .validate(false)
            .map_err(|why| config_error("TLS", why))?;
        if let Some(raw) = spec.akamai.as_deref() {
            profile
                .h2
                .apply_akamai(raw)
                .map_err(|why| config_error("Akamai", why))?;
        }
        if spec.header_order.is_some() {
            profile.meta.header_order = spec.header_order;
        }
        Ok(profile)
    }
}

fn parse_list(field: &str, radix: u32, what: &str) -> Result<Vec<u16>, String> {
    field
        .split([',', '-'])
        .filter(|item| !item.is_empty())
        .map(|item| {
            u16::from_str_radix(item.trim(), radix)
                .map_err(|e| format!("{what} value {item:?} is not a 16-bit number: {e}"))
        })
        .collect()
}

impl TlsProfile {
    fn iana_names(
        &mut self,
        ids: &[u16],
        lookup: fn(u16) -> Option<&'static str>,
        what: &str,
    ) -> Result<Vec<String>, String> {
        let mut names = Vec::with_capacity(ids.len());
        for &id in ids {
            if is_grease(id) {
                self.grease = true;
                continue;
            }
            let name = lookup(id)
                .ok_or_else(|| format!("{what} 0x{id:04x} ({id}) is not in the IANA registry"))?;
            names.push(name.to_owned());
        }
        Ok(names)
    }
}
