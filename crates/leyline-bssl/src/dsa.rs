//! Digital Signatures
//!
//! DSA ensures a message originated from a known sender, and was not modified.
//! DSA uses asymetrical keys and an algorithm to output a signature of the message
//! using the private key that can be validated with the public key but not be generated
//! without the private key.

use foreign_types::{ForeignType, ForeignTypeRef};
use libc::c_uint;
use openssl_macros::corresponds;
use std::fmt;
use std::ptr;

use crate::bn::BigNumRef;
use crate::error::ErrorStack;
use crate::ffi;
use crate::pkey::{HasParams, HasPrivate, HasPublic, Private, Public};
use crate::try_int;
use crate::{cvt, cvt_p};

generic_foreign_type_and_impl_send_sync! {
    type CType = ffi::DSA;
    fn drop = ffi::DSA_free;

    /// Object representing DSA keys.
    ///
    /// A DSA object contains the parameters p, q, and g.  There is a private
    /// and public key.  The values p, g, and q are:
    ///
    /// * `p`: DSA prime parameter
    /// * `q`: DSA sub-prime parameter
    /// * `g`: DSA base parameter
    ///
    /// These values are used to calculate a pair of asymetrical keys used for
    /// signing.
    ///
    /// OpenSSL documentation at [`DSA_new`]
    ///
    /// [`DSA_new`]: https://www.openssl.org/docs/man1.1.0/crypto/DSA_new.html
    ///
    /// # Examples
    ///
    /// ```
    /// use leyline_bssl::dsa::Dsa;
    /// use leyline_bssl::error::ErrorStack;
    /// use leyline_bssl::pkey::Private;
    ///
    /// fn create_dsa() -> Result<Dsa<Private>, ErrorStack> {
    ///     let sign = Dsa::generate(2048)?;
    ///     Ok(sign)
    /// }
    /// # fn main() {
    /// #    create_dsa();
    /// # }
    /// ```
    pub struct Dsa<T>;
    /// Reference to [`Dsa`].
    ///
    /// [`Dsa`]: struct.Dsa.html
    pub struct DsaRef<T>;
}

impl<T> Clone for Dsa<T> {
    fn clone(&self) -> Dsa<T> {
        (**self).to_owned()
    }
}

impl<T> ToOwned for DsaRef<T> {
    type Owned = Dsa<T>;

    fn to_owned(&self) -> Dsa<T> {
        unsafe {
            ffi::DSA_up_ref(self.as_ptr());
            Dsa::from_ptr(self.as_ptr())
        }
    }
}

impl<T> DsaRef<T>
where
    T: HasPublic,
{
    to_pem! {
        /// Serialies the public key into a PEM-encoded SubjectPublicKeyInfo structure.
        ///
        /// The output will have a header of `-----BEGIN PUBLIC KEY-----`.
        #[corresponds(PEM_write_bio_DSA_PUBKEY)]
        public_key_to_pem,
        ffi::PEM_write_bio_DSA_PUBKEY
    }

    to_der! {
        /// Serializes the public key into a DER-encoded SubjectPublicKeyInfo structure.
        #[corresponds(i2d_DSA_PUBKEY)]
        public_key_to_der,
        ffi::i2d_DSA_PUBKEY
    }
}

impl<T> DsaRef<T>
where
    T: HasPrivate,
{
    private_key_to_pem! {
        /// Serializes the private key to a PEM-encoded DSAPrivateKey structure.
        ///
        /// The output will have a header of `-----BEGIN DSA PRIVATE KEY-----`.
        #[corresponds(PEM_write_bio_DSAPrivateKey)]
        private_key_to_pem,
        /// Serializes the private key to a PEM-encoded encrypted DSAPrivateKey structure.
        ///
        /// The output will have a header of `-----BEGIN DSA PRIVATE KEY-----`.
        #[corresponds(PEM_write_bio_DSAPrivateKey)]
        private_key_to_pem_passphrase,
        ffi::PEM_write_bio_DSAPrivateKey
    }
}

impl<T> DsaRef<T>
where
    T: HasParams,
{
    /// Returns the maximum size of the signature output by `self` in bytes.
    #[corresponds(DSA_size)]
    #[must_use]
    pub fn size(&self) -> u32 {
        unsafe { ffi::DSA_size(self.as_ptr()) as u32 }
    }

    /// Returns the DSA prime parameter of `self`.
    #[must_use]
    pub fn p(&self) -> &BigNumRef {
        unsafe {
            let mut p = ptr::null();
            DSA_get0_pqg(self.as_ptr(), &mut p, ptr::null_mut(), ptr::null_mut());
            BigNumRef::from_ptr(p.cast_mut())
        }
    }

    /// Returns the DSA sub-prime parameter of `self`.
    #[must_use]
    pub fn q(&self) -> &BigNumRef {
        unsafe {
            let mut q = ptr::null();
            DSA_get0_pqg(self.as_ptr(), ptr::null_mut(), &mut q, ptr::null_mut());
            BigNumRef::from_ptr(q.cast_mut())
        }
    }

    /// Returns the DSA base parameter of `self`.
    #[must_use]
    pub fn g(&self) -> &BigNumRef {
        unsafe {
            let mut g = ptr::null();
            DSA_get0_pqg(self.as_ptr(), ptr::null_mut(), ptr::null_mut(), &mut g);
            BigNumRef::from_ptr(g.cast_mut())
        }
    }
}

impl Dsa<Private> {
    /// Generate a DSA key pair.
    ///
    /// Calls [`DSA_generate_parameters_ex`] to populate the `p`, `g`, and `q` values.
    /// These values are used to generate the key pair with [`DSA_generate_key`].
    ///
    /// The `bits` parameter corresponds to the length of the prime `p`.
    ///
    /// [`DSA_generate_parameters_ex`]: https://www.openssl.org/docs/man1.1.0/crypto/DSA_generate_parameters_ex.html
    /// [`DSA_generate_key`]: https://www.openssl.org/docs/man1.1.0/crypto/DSA_generate_key.html
    pub fn generate(bits: u32) -> Result<Dsa<Private>, ErrorStack> {
        ffi::init();
        unsafe {
            let dsa = Dsa::from_ptr(cvt_p(ffi::DSA_new())?);
            cvt(ffi::DSA_generate_parameters_ex(
                dsa.0,
                c_uint::from(bits),
                ptr::null(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ))?;
            cvt(ffi::DSA_generate_key(dsa.0))?;
            Ok(dsa)
        }
    }
}

impl Dsa<Public> {
    from_pem! {
        /// Decodes a PEM-encoded SubjectPublicKeyInfo structure containing a DSA key.
        ///
        /// The input should have a header of `-----BEGIN PUBLIC KEY-----`.
        #[corresponds(PEM_read_bio_DSA_PUBKEY)]
        public_key_from_pem,
        Dsa<Public>,
        ffi::PEM_read_bio_DSA_PUBKEY
    }

    from_der! {
        /// Decodes a DER-encoded SubjectPublicKeyInfo structure containing a DSA key.
        #[corresponds(d2i_DSA_PUBKEY)]
        public_key_from_der,
        Dsa<Public>,
        ffi::d2i_DSA_PUBKEY,
        ::libc::c_long
    }
}

impl<T> fmt::Debug for Dsa<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DSA")
    }
}

use crate::ffi::DSA_get0_pqg;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    pub fn test_generate() {
        Dsa::generate(1024).unwrap();
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn clone() {
        let key = Dsa::generate(2048).unwrap();
        drop(key.clone());
    }
}
