use leyline_bssl::ssl::SslRef;

use crate::profile::TlsProfile;
use crate::tls::error::TlsError;

#[derive(Debug, Clone)]
pub(crate) struct HelloOptions {
    ech_grease: bool,
    alps_proto: Option<Vec<u8>>,
    alps_new_codepoint: bool,
    request_trust_anchors: bool,
    key_shares: Option<Vec<u16>>,
    tls12_extensions: bool,
}

impl HelloOptions {
    pub(crate) fn from_tls(tls: &TlsProfile) -> Result<Self, TlsError> {
        Ok(Self {
            ech_grease: tls.ech_grease,
            alps_proto: tls.alps.as_ref().map(|s| s.as_bytes().to_vec()),
            alps_new_codepoint: tls.alps_new_codepoint,
            request_trust_anchors: tls.request_trust_anchors,
            key_shares: key_share_ids(tls)?,
            tls12_extensions: tls.tls12_extensions,
        })
    }

    pub(crate) fn ech_grease(&self) -> bool {
        self.ech_grease
    }

    pub(crate) fn apply(&self, ssl: &mut SslRef, include_alps: bool) -> Result<(), TlsError> {
        if include_alps && let Some(ref alps) = self.alps_proto {
            ssl.add_application_settings(alps)
                .map_err(TlsError::from_stack)?;
            if self.alps_new_codepoint {
                ssl.set_alps_use_new_codepoint(true);
            }
        }

        if self.ech_grease {
            ssl.set_enable_ech_grease(true);
        }

        if let Some(ids) = &self.key_shares {
            ssl.set_client_key_shares(ids)
                .map_err(TlsError::from_stack)?;
        }

        if self.request_trust_anchors {
            ssl.set_requested_trust_anchors(&[]).map_err(|e| {
                TlsError::SslConfig(format!(
                    "profile requires the trust_anchors extension (0xCA34), which BoringSSL \
                     rejected ({e}); the ClientHello JA4 would not match the captured browser"
                ))
            })?;
        }

        if self.tls12_extensions {
            ssl.set_tls12_extensions(true);
        }

        Ok(())
    }
}

fn key_share_ids(tls: &TlsProfile) -> Result<Option<Vec<u16>>, TlsError> {
    let Some(names) = tls.key_shares.as_deref() else {
        return Ok(None);
    };
    let ids = names
        .iter()
        .map(|name| {
            crate::iana::curve_id(name)
                .ok_or_else(|| TlsError::Profile(format!("unknown key share group: {name}")))
        })
        .collect::<Result<Vec<u16>, _>>()?;
    let mut curves = tls
        .curves
        .iter()
        .filter_map(|name| crate::iana::curve_id(name));
    if !ids.iter().all(|id| curves.any(|curve| curve == *id)) {
        return Err(TlsError::Profile(format!(
            "key_shares {names:?} must be an ordered subsequence of curves {:?}",
            tls.curves
        )));
    }
    Ok(Some(ids))
}
