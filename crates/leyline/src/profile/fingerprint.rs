mod akamai;
pub use akamai::akamai_fingerprint;
mod raw;

use crate::error::Error;
use crate::iana::{cipher_name, curve_name, is_grease, sigalg_name};
use crate::profile::{BrowserProfile, Carrier, TlsProfile};
pub(crate) use raw::{JA3_POINT_FORMATS, JA3_RECORD_VERSION, JA4_TCP};
use raw::{Ja3, Ja4};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FingerprintSpec {
    layer: FingerprintLayer,
    base: Option<BrowserProfile>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FingerprintLayer {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    ja4_r: Option<String>,
    #[serde(default)]
    ja3: Option<String>,
    #[serde(default)]
    akamai: Option<String>,
    #[serde(default)]
    header_order: Option<Vec<String>>,
}

impl FingerprintSpec {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn ja3(mut self, raw: impl Into<String>) -> Self {
        self.layer.ja3 = Some(raw.into());
        self
    }

    #[must_use]
    pub fn ja4_r(mut self, raw: impl Into<String>) -> Self {
        self.layer.ja4_r = Some(raw.into());
        self
    }

    #[must_use]
    pub fn akamai(mut self, raw: impl Into<String>) -> Self {
        self.layer.akamai = Some(raw.into());
        self
    }

    #[must_use]
    pub fn header_order<I, S>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.layer.header_order = Some(order.into_iter().map(Into::into).collect());
        self
    }

    #[must_use]
    pub fn base(mut self, profile: BrowserProfile) -> Self {
        self.base = Some(profile);
        self
    }
}

fn config_error(format: &str, why: impl std::fmt::Display) -> Error {
    Error::config(format!("{format} fingerprint: {why}"))
}

impl BrowserProfile {
    pub fn from_fingerprint(spec: FingerprintSpec) -> crate::error::Result<Self> {
        let base = spec.base.unwrap_or_else(|| (*Self::bare_shared()).clone());
        let source = base
            .source()
            .ok_or_else(|| Error::config("fingerprint base profile has no TOML source"))?;
        Self::from_toml(&format!("{source}{}", spec.layer.to_toml()))
    }
}

impl FingerprintLayer {
    fn to_toml(&self) -> String {
        let mut out = String::from("\n[[fingerprint]]\n");
        for (key, value) in [
            ("name", &self.name),
            ("ja4_r", &self.ja4_r),
            ("ja3", &self.ja3),
            ("akamai", &self.akamai),
        ] {
            if let Some(value) = value {
                out += &format!("{key} = {}\n", quote(value));
            }
        }
        if let Some(order) = &self.header_order {
            let names: Vec<String> = order.iter().map(|name| quote(name)).collect();
            out += &format!("header_order = [{}]\n", names.join(", "));
        }
        out
    }

    pub(crate) fn apply(&self, profile: &mut BrowserProfile) -> crate::error::Result<()> {
        if let Some(name) = &self.name {
            profile.meta.name.clone_from(name);
        }
        if self.ja4_r.is_some() || self.ja3.is_some() {
            profile.tls.fingerprint = None;
        }
        if let Some(raw) = self.ja4_r.as_deref() {
            profile
                .tls
                .apply_ja4(raw)
                .map_err(|why| config_error("JA4_r", why))?;
        }
        if let Some(raw) = self.ja3.as_deref() {
            profile
                .tls
                .apply_ja3(raw)
                .map_err(|why| config_error("JA3", why))?;
        }
        profile
            .tls
            .validate(Carrier::Tcp)
            .map_err(|why| config_error("TLS", why))?;
        if let Some(raw) = self.akamai.as_deref() {
            profile
                .h2
                .apply_akamai(raw)
                .map_err(|why| config_error("Akamai", why))?;
        }
        if self.header_order.is_some() {
            profile.meta.header_order.clone_from(&self.header_order);
        }
        Ok(())
    }
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            c if c.is_control() => out += &format!("\\u{:04X}", u32::from(c)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl TlsProfile {
    fn apply_ja3(&mut self, raw: &str) -> Result<(), String> {
        let ja3 = Ja3::parse(raw)?;
        self.ciphers = self.iana_names(&ja3.ciphers, cipher_name, "cipher")?;
        self.curves = self.iana_names(&ja3.curves, curve_name, "curve")?;
        if let Some(shares) = self.key_shares.as_mut() {
            shares.retain(|share| self.curves.contains(share));
        }
        let order = self.apply_extensions(&ja3.extensions)?;
        self.extension_permutation = Some(order);
        self.permute_extensions = false;
        Ok(())
    }

    fn apply_ja4(&mut self, raw: &str) -> Result<(), String> {
        let ja4 = Ja4::parse(raw)?;
        self.ciphers = self.iana_names(&ja4.ciphers, cipher_name, "cipher")?;
        self.sigalgs = self.iana_names(&ja4.sigalgs, sigalg_name, "signature algorithm")?;
        self.apply_extensions(&ja4.extensions)?;
        self.extension_permutation = None;
        Ok(())
    }

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
