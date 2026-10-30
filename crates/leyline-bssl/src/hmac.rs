use crate::cvt;
use crate::error::ErrorStack;
use crate::foreign_types::ForeignTypeRef;
use crate::hash::MessageDigest;
use openssl_macros::corresponds;

foreign_type_and_impl_send_sync! {
    type CType = ffi::HMAC_CTX;
    fn drop = ffi::HMAC_CTX_free;

    pub struct HmacCtx;
}

impl HmacCtxRef {
    /// Configures HmacCtx to use `md` as the hash function and `key` as the key.
    ///
    #[corresponds(HMAC_Init_ex)]
    pub fn init(&mut self, key: &[u8], md: &MessageDigest) -> Result<(), ErrorStack> {
        ffi::init();

        unsafe {
            cvt(ffi::HMAC_Init_ex(
                self.as_ptr(),
                key.as_ptr().cast(),
                key.len(),
                md.as_ptr(),
                core::ptr::null_mut(),
            ))
        }
    }
}

/// Provides an init-update-finalize API for HMAC.
pub struct Hmac(*mut ffi::HMAC_CTX);

impl Hmac {
    /// Creates a new HMAC object with the given key and hash algorithm.
    pub fn init(key: &[u8], md: &MessageDigest) -> Result<Hmac, ErrorStack> {
        ffi::init();

        let ctx = unsafe {
            let ctx = ffi::HMAC_CTX_new();
            cvt(ffi::HMAC_Init_ex(
                ctx,
                key.as_ptr().cast(),
                key.len(),
                md.as_ptr(),
                core::ptr::null_mut(),
            ))?;
            ctx
        };

        Ok(Hmac(ctx))
    }

    /// Updates the HMAC input.
    pub fn update(&mut self, data: &[u8]) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::HMAC_Update(self.0, data.as_ptr().cast(), data.len())) }
    }

    /// Finalizes the HMAC and returns the output.
    pub fn finalize(self) -> Result<Vec<u8>, ErrorStack> {
        let out_len = unsafe { ffi::HMAC_size(self.0) };
        let mut out = vec![0; out_len];
        unsafe {
            cvt(ffi::HMAC_Final(
                self.0,
                out.as_mut_ptr().cast(),
                core::ptr::null_mut(),
            ))?;
        }
        Ok(out)
    }
}

impl Drop for Hmac {
    fn drop(&mut self) {
        unsafe { ffi::HMAC_CTX_free(self.0) }
    }
}

#[cfg(test)]
mod tests {}
