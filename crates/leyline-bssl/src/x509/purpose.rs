use foreign_types::ForeignTypeRef;
use libc::c_int;
use openssl_macros::corresponds;

use crate::error::ErrorStack;
use crate::x509::X509StoreContextRef;
use crate::{cvt, ffi};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X509Purpose(c_int);

impl X509Purpose {
    pub const SSL_SERVER: X509Purpose = X509Purpose(ffi::X509_PURPOSE_SSL_SERVER as c_int);
}

impl X509StoreContextRef {
    #[corresponds(X509_STORE_CTX_set_purpose)]
    pub fn set_purpose(&mut self, purpose: X509Purpose) -> Result<(), ErrorStack> {
        // SAFETY: `self` is a live X509_STORE_CTX, and the call reads only the integer purpose.
        unsafe { cvt(ffi::X509_STORE_CTX_set_purpose(self.as_ptr(), purpose.0)).map(|_| ()) }
    }
}
