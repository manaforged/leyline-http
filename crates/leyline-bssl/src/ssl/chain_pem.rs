use std::ffi::c_int;
use std::ptr;

use foreign_types::ForeignType;
use openssl_macros::corresponds;

use super::SslContextBuilder;
use crate::error::ErrorStack;
use crate::x509::X509;
use crate::{cvt, cvt_p, ffi};

impl SslContextBuilder {
    #[corresponds(SSL_CTX_use_certificate_chain_file)]
    pub fn set_certificate_chain_pem(&mut self, pem: &[u8]) -> Result<(), ErrorStack> {
        self.ctx.check_x509();
        let bio = crate::bio::MemBioSlice::new(pem)?;
        // SAFETY: `bio` reads from `pem`, which outlives these calls, and every certificate returned by the PEM readers is owned by an `X509` that frees it.
        unsafe {
            ffi::init();
            let leaf = cvt_p(ffi::PEM_read_bio_X509_AUX(
                bio.as_ptr(),
                ptr::null_mut(),
                None,
                ptr::null_mut(),
            ))
            .map(|cert| X509::from_ptr(cert))?;
            cvt(ffi::SSL_CTX_use_certificate(self.as_ptr(), leaf.as_ptr()))?;
            cvt(ffi::SSL_CTX_clear_chain_certs(self.as_ptr()))?;
            loop {
                let next =
                    ffi::PEM_read_bio_X509(bio.as_ptr(), ptr::null_mut(), None, ptr::null_mut());
                if next.is_null() {
                    break;
                }
                let cert = X509::from_ptr(next);
                cvt(ffi::SSL_CTX_add1_chain_cert(self.as_ptr(), cert.as_ptr()))?;
            }
            let last = ffi::ERR_peek_last_error();
            let pem_lib = c_int::try_from(ffi::ERR_LIB_PEM.0).unwrap_or(c_int::MAX);
            if ffi::ERR_GET_LIB(last) == pem_lib
                && ffi::ERR_GET_REASON(last) == ffi::PEM_R_NO_START_LINE
            {
                ffi::ERR_clear_error();
                return Ok(());
            }
            Err(ErrorStack::get())
        }
    }
}
