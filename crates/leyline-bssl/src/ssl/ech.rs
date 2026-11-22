use std::ffi::{c_int, CString};
use std::ptr;
use std::slice;

use foreign_types::{ForeignType, ForeignTypeRef};
use openssl_macros::corresponds;

use crate::error::ErrorStack;
use crate::hpke::HpkeKeyRef;
use crate::ssl::{SslContextBuilder, SslRef};
use crate::{cvt, cvt_p, ffi};

pub struct SslEchKeysBuilder {
    keys: SslEchKeys,
}

impl SslEchKeysBuilder {
    pub fn new() -> Result<SslEchKeysBuilder, ErrorStack> {
        unsafe {
            ffi::init();
            let keys = cvt_p(ffi::SSL_ECH_KEYS_new())?;

            Ok(SslEchKeysBuilder::from_ptr(keys))
        }
    }

    pub unsafe fn from_ptr(keys: *mut ffi::SSL_ECH_KEYS) -> Self {
        Self {
            keys: unsafe { SslEchKeys::from_ptr(keys) },
        }
    }

    #[corresponds(SSL_ECH_KEYS_add)]
    pub fn add(
        &mut self,
        is_retry_config: bool,
        ech_config: &[u8],
        key: &HpkeKeyRef,
    ) -> Result<(), ErrorStack> {
        // SAFETY: keys and key are valid BoringSSL handles and ech_config is readable for its length.
        unsafe {
            cvt(ffi::SSL_ECH_KEYS_add(
                self.keys.as_ptr(),
                c_int::from(is_retry_config),
                ech_config.as_ptr(),
                ech_config.len(),
                key.as_ptr(),
            ))
        }
    }

    pub fn build(self) -> SslEchKeys {
        self.keys
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::SSL_ECH_KEYS;
    fn drop = ffi::SSL_ECH_KEYS_free;

    pub struct SslEchKeys;
}

impl SslEchKeys {
    pub fn builder() -> Result<SslEchKeysBuilder, ErrorStack> {
        SslEchKeysBuilder::new()
    }
}

impl SslEchKeysRef {
    #[corresponds(SSL_ECH_KEYS_marshal_retry_configs)]
    pub fn marshal_retry_configs(&self) -> Result<Vec<u8>, ErrorStack> {
        let mut out: *mut u8 = ptr::null_mut();
        let mut out_len: usize = 0;
        // SAFETY: self is valid; BoringSSL allocates out with OPENSSL_malloc on success.
        unsafe {
            cvt(ffi::SSL_ECH_KEYS_marshal_retry_configs(
                self.as_ptr(),
                &mut out,
                &mut out_len,
            ))?;
            let copy = if out.is_null() {
                Vec::new()
            } else {
                let bytes = slice::from_raw_parts(out, out_len).to_vec();
                ffi::OPENSSL_free(out.cast());
                bytes
            };
            Ok(copy)
        }
    }
}

/// Marshals a single RFC 9460 `ECHConfig` for `key` and `public_name`.
#[corresponds(SSL_marshal_ech_config)]
pub fn marshal_ech_config(
    config_id: u8,
    key: &HpkeKeyRef,
    public_name: &str,
    max_name_len: usize,
) -> Result<Vec<u8>, ErrorStack> {
    let name = CString::new(public_name)
        .map_err(|_| ErrorStack::internal_error_str("ECH public_name contains interior NUL"))?;
    let mut out: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;
    // SAFETY: key and name are valid; BoringSSL allocates out with OPENSSL_malloc on success.
    unsafe {
        cvt(ffi::SSL_marshal_ech_config(
            &mut out,
            &mut out_len,
            config_id,
            key.as_ptr(),
            name.as_ptr(),
            max_name_len,
        ))?;
        let copy = if out.is_null() {
            Vec::new()
        } else {
            let bytes = slice::from_raw_parts(out, out_len).to_vec();
            ffi::OPENSSL_free(out.cast());
            bytes
        };
        Ok(copy)
    }
}

/// Returns true when `stack` contains BoringSSL's `SSL_R_ECH_REJECTED` error.
#[must_use]
pub fn is_ech_rejected(stack: &ErrorStack) -> bool {
    stack.errors().iter().any(|err| {
        err.library_code() == ffi::ERR_LIB_SSL.0 as c_int
            && err.reason_code() == ffi::SSL_R_ECH_REJECTED
    })
}

impl SslContextBuilder {
    #[corresponds(SSL_CTX_set1_ech_keys)]
    pub fn set_ech_keys(&mut self, keys: &SslEchKeysRef) -> Result<(), ErrorStack> {
        // SAFETY: ctx and keys are valid BoringSSL handles; SSL_CTX_set1_ech_keys increments keys refcount.
        unsafe { cvt(ffi::SSL_CTX_set1_ech_keys(self.as_ptr(), keys.as_ptr())) }
    }
}

impl SslRef {
    #[corresponds(SSL_set1_ech_config_list)]
    pub fn set_ech_config_list(&mut self, ech_config_list: &[u8]) -> Result<(), ErrorStack> {
        // SAFETY: ssl is valid and ech_config_list is readable for its length.
        unsafe {
            cvt(ffi::SSL_set1_ech_config_list(
                self.as_ptr(),
                ech_config_list.as_ptr(),
                ech_config_list.len(),
            ))
        }
    }

    #[corresponds(SSL_ech_accepted)]
    #[must_use]
    pub fn ech_accepted(&self) -> bool {
        // SAFETY: ssl is valid.
        unsafe { ffi::SSL_ech_accepted(self.as_ptr()) == 1 }
    }

    #[corresponds(SSL_get0_ech_retry_configs)]
    #[must_use]
    pub fn ech_retry_configs(&self) -> Option<&[u8]> {
        let mut ptr: *const u8 = ptr::null();
        let mut len: usize = 0;
        // SAFETY: ssl is valid; returned slice borrows memory owned by ssl.
        unsafe {
            ffi::SSL_get0_ech_retry_configs(self.as_ptr(), &mut ptr, &mut len);
            if ptr.is_null() || len == 0 {
                None
            } else {
                Some(slice::from_raw_parts(ptr, len))
            }
        }
    }
}
