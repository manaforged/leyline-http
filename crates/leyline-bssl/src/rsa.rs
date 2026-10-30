//! Rivest–Shamir–Adleman cryptosystem
//!
//! RSA is one of the earliest asymmetric public key encryption schemes.
//! Like many other cryptosystems, RSA relies on the presumed difficulty of a hard
//! mathematical problem, namely factorization of the product of two large prime
//! numbers. At the moment there does not exist an algorithm that can factor such
//! large numbers in reasonable time. RSA is used in a wide variety of
//! applications including digital signatures and key exchanges such as
//! establishing a TLS/SSL connection.
//!
//! The RSA acronym is derived from the first letters of the surnames of the
//! algorithm's founding trio.
use foreign_types::{ForeignType, ForeignTypeRef};
use libc::c_int;
use openssl_macros::corresponds;
use std::fmt;
use std::mem;
use std::ptr;

use crate::bn::{BigNum, BigNumRef};
use crate::error::ErrorStack;
use crate::ffi;
use crate::pkey::{HasPrivate, HasPublic, Private, Public};
use crate::try_int;
use crate::{cvt, cvt_p};

/// Type of encryption padding to use.
///
/// Random length padding is primarily used to prevent attackers from
/// predicting or knowing the exact length of a plaintext message that
/// can possibly lead to breaking encryption.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Padding(c_int);

impl Padding {
    pub const NONE: Padding = Padding(ffi::RSA_NO_PADDING);

    /// Creates a `Padding` from an integer representation.
    #[must_use]
    pub fn from_raw(value: c_int) -> Padding {
        Padding(value)
    }

    /// Returns the integer representation of `Padding`.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

generic_foreign_type_and_impl_send_sync! {
    type CType = ffi::RSA;
    fn drop = ffi::RSA_free;

    /// An RSA key.
    pub struct Rsa<T>;

    /// Reference to `RSA`
    pub struct RsaRef<T>;
}

impl<T> Clone for Rsa<T> {
    fn clone(&self) -> Rsa<T> {
        (**self).to_owned()
    }
}

impl<T> ToOwned for RsaRef<T> {
    type Owned = Rsa<T>;

    fn to_owned(&self) -> Rsa<T> {
        unsafe {
            ffi::RSA_up_ref(self.as_ptr());
            Rsa::from_ptr(self.as_ptr())
        }
    }
}

impl<T> RsaRef<T>
where
    T: HasPrivate,
{
    private_key_to_pem! {
        /// Serializes the private key to a PEM-encoded PKCS#1 RSAPrivateKey structure.
        ///
        /// The output will have a header of `-----BEGIN RSA PRIVATE KEY-----`.
        #[corresponds(PEM_write_bio_RSAPrivateKey)]
        private_key_to_pem,
        /// Serializes the private key to a PEM-encoded encrypted PKCS#1 RSAPrivateKey structure.
        ///
        /// The output will have a header of `-----BEGIN RSA PRIVATE KEY-----`.
        #[corresponds(PEM_write_bio_RSAPrivateKey)]
        private_key_to_pem_passphrase,
        ffi::PEM_write_bio_RSAPrivateKey
    }

    to_der! {
        /// Serializes the private key to a DER-encoded PKCS#1 RSAPrivateKey structure.
        #[corresponds(i2d_RSAPrivateKey)]
        private_key_to_der,
        ffi::i2d_RSAPrivateKey
    }

    /// Returns a reference to the private exponent of the key.
    #[corresponds(RSA_get0_key)]
    #[must_use]
    pub fn d(&self) -> &BigNumRef {
        unsafe {
            let mut d = ptr::null();
            RSA_get0_key(self.as_ptr(), ptr::null_mut(), ptr::null_mut(), &mut d);
            BigNumRef::from_ptr(d.cast_mut())
        }
    }

    /// Returns a reference to the first factor of the exponent of the key.
    #[corresponds(RSA_get0_factors)]
    #[must_use]
    pub fn p(&self) -> Option<&BigNumRef> {
        unsafe {
            let mut p = ptr::null();
            RSA_get0_factors(self.as_ptr(), &mut p, ptr::null_mut());
            if p.is_null() {
                None
            } else {
                Some(BigNumRef::from_ptr(p.cast_mut()))
            }
        }
    }

    /// Returns a reference to the second factor of the exponent of the key.
    #[corresponds(RSA_get0_factors)]
    #[must_use]
    pub fn q(&self) -> Option<&BigNumRef> {
        unsafe {
            let mut q = ptr::null();
            RSA_get0_factors(self.as_ptr(), ptr::null_mut(), &mut q);
            if q.is_null() {
                None
            } else {
                Some(BigNumRef::from_ptr(q.cast_mut()))
            }
        }
    }
}

impl<T> RsaRef<T>
where
    T: HasPublic,
{
    to_pem! {
        /// Serializes the public key into a PEM-encoded SubjectPublicKeyInfo structure.
        ///
        /// The output will have a header of `-----BEGIN PUBLIC KEY-----`.
        #[corresponds(PEM_write_bio_RSA_PUBKEY)]
        public_key_to_pem,
        ffi::PEM_write_bio_RSA_PUBKEY
    }

    to_der! {
        /// Serializes the public key into a DER-encoded SubjectPublicKeyInfo structure.
        #[corresponds(i2d_RSA_PUBKEY)]
        public_key_to_der,
        ffi::i2d_RSA_PUBKEY
    }

    to_pem! {
        /// Serializes the public key into a PEM-encoded PKCS#1 RSAPublicKey structure.
        ///
        /// The output will have a header of `-----BEGIN RSA PUBLIC KEY-----`.
        #[corresponds(PEM_write_bio_RSAPublicKey)]
        public_key_to_pem_pkcs1,
        ffi::PEM_write_bio_RSAPublicKey
    }

    to_der! {
        /// Serializes the public key into a DER-encoded PKCS#1 RSAPublicKey structure.
        #[corresponds(i2d_RSAPublicKey)]
        public_key_to_der_pkcs1,
        ffi::i2d_RSAPublicKey
    }

    /// Returns the size of the modulus in bytes.
    #[corresponds(RSA_size)]
    #[allow(clippy::unnecessary_cast)]
    #[must_use]
    pub fn size(&self) -> u32 {
        unsafe { ffi::RSA_size(self.as_ptr()) as u32 }
    }

    /// Returns a reference to the modulus of the key.
    #[corresponds(RSA_get0_key)]
    #[must_use]
    pub fn n(&self) -> &BigNumRef {
        unsafe {
            let mut n = ptr::null();
            RSA_get0_key(self.as_ptr(), &mut n, ptr::null_mut(), ptr::null_mut());
            BigNumRef::from_ptr(n.cast_mut())
        }
    }

    /// Returns a reference to the public exponent of the key.
    #[corresponds(RSA_get0_key)]
    #[must_use]
    pub fn e(&self) -> &BigNumRef {
        unsafe {
            let mut e = ptr::null();
            RSA_get0_key(self.as_ptr(), ptr::null_mut(), &mut e, ptr::null_mut());
            BigNumRef::from_ptr(e.cast_mut())
        }
    }
}

impl Rsa<Public> {
    from_pem! {
        /// Decodes a PEM-encoded SubjectPublicKeyInfo structure containing an RSA key.
        ///
        /// The input should have a header of `-----BEGIN PUBLIC KEY-----`.
        #[corresponds(PEM_read_bio_RSA_PUBKEY)]
        public_key_from_pem,
        Rsa<Public>,
        ffi::PEM_read_bio_RSA_PUBKEY
    }

    from_pem! {
        /// Decodes a PEM-encoded PKCS#1 RSAPublicKey structure.
        ///
        /// The input should have a header of `-----BEGIN RSA PUBLIC KEY-----`.
        #[corresponds(PEM_read_bio_RSAPublicKey)]
        public_key_from_pem_pkcs1,
        Rsa<Public>,
        ffi::PEM_read_bio_RSAPublicKey
    }

    from_der! {
        /// Decodes a DER-encoded SubjectPublicKeyInfo structure containing an RSA key.
        #[corresponds(d2i_RSA_PUBKEY)]
        public_key_from_der,
        Rsa<Public>,
        ffi::d2i_RSA_PUBKEY,
        ::libc::c_long
    }

    from_der! {
        /// Decodes a DER-encoded PKCS#1 RSAPublicKey structure.
        #[corresponds(d2i_RSAPublicKey)]
        public_key_from_der_pkcs1,
        Rsa<Public>,
        ffi::d2i_RSAPublicKey,
        ::libc::c_long
    }
}

pub struct RsaPrivateKeyBuilder {
    rsa: Rsa<Private>,
}

impl RsaPrivateKeyBuilder {
    /// Creates a new `RsaPrivateKeyBuilder`.
    ///
    /// `n` is the modulus common to both public and private key.
    /// `e` is the public exponent and `d` is the private exponent.
    #[corresponds(RSA_new)]
    pub fn new(n: BigNum, e: BigNum, d: BigNum) -> Result<RsaPrivateKeyBuilder, ErrorStack> {
        unsafe {
            let rsa = cvt_p(ffi::RSA_new())?;
            cvt(RSA_set0_key(rsa, n.as_ptr(), e.as_ptr(), d.as_ptr()))?;
            mem::forget((n, e, d));
            Ok(RsaPrivateKeyBuilder {
                rsa: Rsa::from_ptr(rsa),
            })
        }
    }

    /// Returns the Rsa key.
    #[must_use]
    pub fn build(self) -> Rsa<Private> {
        self.rsa
    }
}

impl Rsa<Private> {
    /// Generates a public/private key pair with the specified size.
    ///
    /// The public exponent will be 65537.
    #[corresponds(RSA_generate_key_ex)]
    pub fn generate(bits: u32) -> Result<Rsa<Private>, ErrorStack> {
        let e = BigNum::from_u32(ffi::RSA_F4 as u32)?;
        Rsa::generate_with_e(bits, &e)
    }

    /// Generates a public/private key pair with the specified size and a custom exponent.
    ///
    /// Unless you have specific needs and know what you're doing, use `Rsa::generate` instead.
    #[corresponds(RSA_generate_key_ex)]
    pub fn generate_with_e(bits: u32, e: &BigNumRef) -> Result<Rsa<Private>, ErrorStack> {
        unsafe {
            let rsa = Rsa::from_ptr(cvt_p(ffi::RSA_new())?);
            cvt(ffi::RSA_generate_key_ex(
                rsa.0,
                bits as c_int,
                e.as_ptr(),
                ptr::null_mut(),
            ))?;
            Ok(rsa)
        }
    }

    private_key_from_pem! {
        /// Deserializes a private key from a PEM-encoded PKCS#1 RSAPrivateKey structure.
        #[corresponds(PEM_read_bio_RSAPrivateKey)]
        private_key_from_pem,

        /// Deserializes a private key from a PEM-encoded encrypted PKCS#1 RSAPrivateKey structure.
        #[corresponds(PEM_read_bio_RSAPrivateKey)]
        private_key_from_pem_passphrase,

        /// Deserializes a private key from a PEM-encoded encrypted PKCS#1 RSAPrivateKey structure.
        ///
        /// The callback should fill the password into the provided buffer and return its length.
        #[corresponds(PEM_read_bio_RSAPrivateKey)]
        private_key_from_pem_callback,
        Rsa<Private>,
        ffi::PEM_read_bio_RSAPrivateKey
    }

    from_der! {
        /// Decodes a DER-encoded PKCS#1 RSAPrivateKey structure.
        #[corresponds(d2i_RSAPrivateKey)]
        private_key_from_der,
        Rsa<Private>,
        ffi::d2i_RSAPrivateKey,
        ::libc::c_long
    }
}

impl<T> fmt::Debug for Rsa<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Rsa")
    }
}

use crate::ffi::{RSA_get0_factors, RSA_get0_key, RSA_set0_key};

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_from_password() {
        let key = include_bytes!("../test/rsa-encrypted.pem");
        Rsa::private_key_from_pem_passphrase(key, b"mypass").unwrap();
    }

    #[test]
    fn test_from_password_callback() {
        let mut password_queried = false;
        let key = include_bytes!("../test/rsa-encrypted.pem");
        Rsa::private_key_from_pem_callback(key, |password| {
            password_queried = true;
            password[..6].copy_from_slice(b"mypass");
            Ok(6)
        })
        .unwrap();

        assert!(password_queried);
    }

    #[test]
    fn test_public_key_from_pem_pkcs1() {
        let key = include_bytes!("../test/pkcs1.pem.pub");
        Rsa::public_key_from_pem_pkcs1(key).unwrap();
    }

    #[test]
    #[should_panic]
    fn test_public_key_from_pem_pkcs1_file_panic() {
        let key = include_bytes!("../test/key.pem.pub");
        Rsa::public_key_from_pem_pkcs1(key).unwrap();
    }

    #[test]
    fn test_public_key_to_pem_pkcs1() {
        let keypair = super::Rsa::generate(512).unwrap();
        let pubkey_pem = keypair.public_key_to_pem_pkcs1().unwrap();
        super::Rsa::public_key_from_pem_pkcs1(&pubkey_pem).unwrap();
    }

    #[test]
    #[should_panic]
    fn test_public_key_from_pem_pkcs1_generate_panic() {
        let keypair = super::Rsa::generate(512).unwrap();
        let pubkey_pem = keypair.public_key_to_pem().unwrap();
        super::Rsa::public_key_from_pem_pkcs1(&pubkey_pem).unwrap();
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn clone() {
        let key = Rsa::generate(2048).unwrap();
        drop(key.clone());
    }

    #[test]
    fn generate_with_e() {
        let e = BigNum::from_u32(0x10001).unwrap();
        Rsa::generate_with_e(2048, &e).unwrap();
    }
}
