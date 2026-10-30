use crate::ffi;
use foreign_types::ForeignType;

use crate::cvt_p;
use crate::error::ErrorStack;

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
