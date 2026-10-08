use crate::profile::TlsProfile;
use crate::profile::extension::{PADDING, PRE_SHARED_KEY};

impl TlsProfile {
    pub(crate) fn extension_ids(&self) -> Vec<u16> {
        let mut ids = match self.extension_permutation.as_deref() {
            Some(order) => order.to_vec(),
            None => self
                .advertised_extensions(false)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
        };
        if self.padding {
            ids.push(PADDING);
        }
        ids
    }

    pub(super) fn validate(&self, quic: bool) -> Result<(), String> {
        self.validate_tail(quic)?;
        let Some(order) = self.extension_permutation.as_deref() else {
            return Ok(());
        };
        if order.is_empty() {
            return Err(
                "extension_permutation is empty; omit the key to accept BoringSSL's \
                        default extension order"
                    .to_string(),
            );
        }

        let advertised = self.advertised_extensions(quic);
        for (index, id) in order.iter().enumerate() {
            if *id == PRE_SHARED_KEY {
                return Err(format!(
                    "extension_permutation entry {index} is pre_shared_key (0x{PRE_SHARED_KEY:04x}), \
                     which TLS 1.3 fixes last and BoringSSL will not reposition; drop it from the list"
                ));
            }
            if order[..index].contains(id) {
                return Err(format!(
                    "extension_permutation repeats 0x{id:04x} at entry {index}; BoringSSL rejects a \
                     duplicated extension ID, leaving the ClientHello in its default order"
                ));
            }
            if !advertised.iter().any(|(known, _)| known == id) {
                return Err(format!(
                    "extension_permutation entry {index} is 0x{id:04x}, which this profile never \
                     advertises; order only the extensions the [tls] block turns on"
                ));
            }
        }

        let missing: Vec<String> = advertised
            .iter()
            .filter(|(id, _)| !order.contains(id))
            .map(|(id, name)| format!("{name} (0x{id:04x})"))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "extension_permutation omits {}; BoringSSL appends whatever the list leaves out, in \
                 its own order, so the tail of the ClientHello would not match the captured browser. \
                 Add each one at its captured position",
                missing.join(", ")
            ));
        }

        Ok(())
    }

    fn validate_tail(&self, quic: bool) -> Result<(), String> {
        if self.extension_tail.is_empty() {
            return Ok(());
        }
        if !quic || self.grease || !self.permute_extensions || self.extension_permutation.is_some()
        {
            return Err(
                "extension_tail belongs in [h3.tls] with permute_extensions = true, grease = false \
                 and no extension_permutation; it pins the last extensions of a shuffled ClientHello"
                    .to_string(),
            );
        }
        let advertised = self.advertised_extensions(quic);
        for (index, id) in self.extension_tail.iter().enumerate() {
            if *id == PRE_SHARED_KEY || self.extension_tail[..index].contains(id) {
                return Err(format!(
                    "extension_tail entry {index} (0x{id:04x}) is pre_shared_key or a repeat"
                ));
            }
            if !advertised.iter().any(|(known, _)| known == id) {
                return Err(format!(
                    "extension_tail entry {index} is 0x{id:04x}, which this profile never advertises"
                ));
            }
        }
        Ok(())
    }
}
