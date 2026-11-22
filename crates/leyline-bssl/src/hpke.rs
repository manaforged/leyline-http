use crate::error::ErrorStack;
use crate::{cvt, cvt_p, ffi};
use foreign_types::ForeignType;

foreign_type_and_impl_send_sync! {
    type CType = ffi::EVP_HPKE_KEY;
    fn drop = ffi::EVP_HPKE_KEY_free;

    pub struct HpkeKey;
}

impl HpkeKey {
    /// Generates a fresh X25519-HKDF-SHA256 HPKE keypair.
    pub fn generate_x25519() -> Result<Self, ErrorStack> {
        // SAFETY: BoringSSL allocates an owned EVP_HPKE_KEY and initializes it with the static X25519 KEM.
        unsafe {
            let ptr = cvt_p(ffi::EVP_HPKE_KEY_new())?;
            let key = Self::from_ptr(ptr);
            cvt(ffi::EVP_HPKE_KEY_generate(
                key.as_ptr(),
                ffi::EVP_hpke_x25519_hkdf_sha256(),
            ))?;
            Ok(key)
        }
    }
}
