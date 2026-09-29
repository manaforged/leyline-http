use foreign_types::ForeignTypeRef;
use openssl_macros::corresponds;

use crate::error::ErrorStack;
use crate::{cvt, ffi};

use super::SslRef;

impl SslRef {
    #[corresponds(SSL_set1_client_key_shares)]
    pub fn set_client_key_shares(&mut self, group_ids: &[u16]) -> Result<(), ErrorStack> {
        // SAFETY: `self` is a live SSL handle and `group_ids` is valid for `group_ids.len()` reads.
        unsafe {
            cvt(ffi::SSL_set1_client_key_shares(
                self.as_ptr(),
                group_ids.as_ptr(),
                group_ids.len(),
            ))
        }
    }
}
