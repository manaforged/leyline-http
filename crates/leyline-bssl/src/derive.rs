//! Shared secret derivation.
use crate::ffi;
use foreign_types::ForeignTypeRef;
use openssl_macros::corresponds;
use std::marker::PhantomData;
use std::ptr;

use crate::error::ErrorStack;
use crate::pkey::{HasPrivate, PKeyRef};
use crate::{cvt, cvt_p};

/// A type used to derive a shared secret between two keys.
pub struct Deriver<'a>(*mut ffi::EVP_PKEY_CTX, PhantomData<&'a ()>);

unsafe impl Sync for Deriver<'_> {}
unsafe impl Send for Deriver<'_> {}

impl Drop for Deriver<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::EVP_PKEY_CTX_free(self.0);
        }
    }
}

#[allow(clippy::len_without_is_empty)]
impl<'a> Deriver<'a> {
    /// Creates a new `Deriver` using the provided private key.
    #[corresponds(EVP_PKEY_derive_init)]
    pub fn new<T>(key: &'a PKeyRef<T>) -> Result<Deriver<'a>, ErrorStack>
    where
        T: HasPrivate,
    {
        unsafe {
            cvt_p(ffi::EVP_PKEY_CTX_new(key.as_ptr(), ptr::null_mut()))
                .map(|p| Deriver(p, PhantomData))
                .and_then(|ctx| cvt(ffi::EVP_PKEY_derive_init(ctx.0)).map(|_| ctx))
        }
    }

    /// Returns the size of the shared secret.
    ///
    /// It can be used to size the buffer passed to [`Deriver::derive`].
    #[corresponds(EVP_PKEY_derive)]
    pub fn len(&mut self) -> Result<usize, ErrorStack> {
        unsafe {
            let mut len = 0;
            cvt(ffi::EVP_PKEY_derive(self.0, ptr::null_mut(), &mut len)).map(|_| len)
        }
    }

    /// Derives a shared secret between the two keys, writing it into the buffer.
    ///
    /// Returns the number of bytes written.
    #[corresponds(EVP_PKEY_derive)]
    pub fn derive(&mut self, buf: &mut [u8]) -> Result<usize, ErrorStack> {
        let mut len = buf.len();
        unsafe { cvt(ffi::EVP_PKEY_derive(self.0, buf.as_mut_ptr(), &mut len)).map(|_| len) }
    }
}

#[cfg(test)]
mod test {}
