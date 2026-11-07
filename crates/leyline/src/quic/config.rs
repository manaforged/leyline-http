use crate::error::Error;
use crate::profile::{BrowserProfile, H3Profile, PseudoOrder, TlsProfile, parse_pseudo_order};

const DEFAULT_PSEUDO_ORDER: [PseudoOrder; 4] = [
    PseudoOrder::Method,
    PseudoOrder::Scheme,
    PseudoOrder::Authority,
    PseudoOrder::Path,
];

/// The QUIC transport parameters and limits for one HTTP/3 connection, derived from a browser profile.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H3Config {
    pub(crate) h3: H3Profile,
    pub(crate) tls: TlsProfile,
    pub(crate) pseudo_order: [PseudoOrder; 4],
}

impl H3Config {
    /// Builds the config from the `[h3]` table of a browser profile, and fails when the profile has none.
    pub fn from_profile(profile: &BrowserProfile) -> Result<Self, Error> {
        let h3 = profile.h3.as_ref().ok_or_else(|| {
            Error::config(format!(
                "profile {} has no [h3] table, so it has no HTTP/3 transport",
                profile.meta.name
            ))
        })?;
        let pseudo_order = match &h3.pseudo_order {
            Some(tokens) => parse_pseudo_order(tokens, "H3")?,
            None => DEFAULT_PSEUDO_ORDER,
        };
        Ok(Self {
            tls: h3.tls.clone().unwrap_or_else(|| profile.tls.clone()),
            h3: h3.clone(),
            pseudo_order,
        })
    }

    /// Returns the `[h3]` profile table this config applies.
    #[cfg(feature = "bench-internals")]
    pub fn h3(&self) -> &H3Profile {
        &self.h3
    }
}
