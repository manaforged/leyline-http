use foreign_types::{ForeignType, ForeignTypeRef};
use libc::c_int;
use openssl_macros::corresponds;
use std::fmt;
use std::mem;
use std::ptr;

use crate::ec::EcKey;
use crate::error::ErrorStack;
use crate::ffi;
use crate::rsa::Rsa;
use crate::try_int;
use crate::{cvt, cvt_p};

pub enum Params {}

pub enum Public {}

pub enum Private {}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Id(c_int);

impl Id {
    pub const RSA: Id = Id(ffi::EVP_PKEY_RSA);
    pub const RSAPSS: Id = Id(ffi::EVP_PKEY_RSA_PSS);
    pub const DSA: Id = Id(ffi::EVP_PKEY_DSA);
    pub const DH: Id = Id(ffi::EVP_PKEY_DH);
    pub const EC: Id = Id(ffi::EVP_PKEY_EC);
    pub const ED25519: Id = Id(ffi::EVP_PKEY_ED25519);
    pub const ED448: Id = Id(ffi::EVP_PKEY_ED448);
    pub const X25519: Id = Id(ffi::EVP_PKEY_X25519);
    pub const X448: Id = Id(ffi::EVP_PKEY_X448);

    #[must_use]
    pub fn from_raw(value: c_int) -> Id {
        Id(value)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

#[allow(clippy::missing_safety_doc)]
pub unsafe trait HasParams {}

unsafe impl HasParams for Params {}

unsafe impl<T> HasParams for T where T: HasPublic {}

#[allow(clippy::missing_safety_doc)]
pub unsafe trait HasPublic {}

unsafe impl HasPublic for Public {}

unsafe impl<T> HasPublic for T where T: HasPrivate {}

#[allow(clippy::missing_safety_doc)]
pub unsafe trait HasPrivate {}

unsafe impl HasPrivate for Private {}

generic_foreign_type_and_impl_send_sync! {
    type CType = ffi::EVP_PKEY;
    fn drop = ffi::EVP_PKEY_free;

    pub struct PKey<T>;
    pub struct PKeyRef<T>;
}

impl<T> ToOwned for PKeyRef<T> {
    type Owned = PKey<T>;

    fn to_owned(&self) -> PKey<T> {
        unsafe {
            EVP_PKEY_up_ref(self.as_ptr());
            PKey::from_ptr(self.as_ptr())
        }
    }
}

impl<T> PKeyRef<T> {
    #[corresponds(EVP_PKEY_get1_RSA)]
    pub fn rsa(&self) -> Result<Rsa<T>, ErrorStack> {
        unsafe {
            let rsa = cvt_p(ffi::EVP_PKEY_get1_RSA(self.as_ptr()))?;
            Ok(Rsa::from_ptr(rsa))
        }
    }

    #[corresponds(EVP_PKEY_id)]
    #[must_use]
    pub fn id(&self) -> Id {
        unsafe { Id::from_raw(ffi::EVP_PKEY_id(self.as_ptr())) }
    }

    #[corresponds(EVP_PKEY_size)]
    #[must_use]
    pub fn size(&self) -> usize {
        unsafe { ffi::EVP_PKEY_size(self.as_ptr()) as usize }
    }
}

impl<T> PKeyRef<T>
where
    T: HasPublic,
{
    to_pem! {
        #[corresponds(PEM_write_bio_PUBKEY)]
        public_key_to_pem,
        ffi::PEM_write_bio_PUBKEY
    }

    to_der! {
        #[corresponds(i2d_PUBKEY)]
        public_key_to_der,
        ffi::i2d_PUBKEY
    }

    #[must_use]
    pub fn bits(&self) -> u32 {
        unsafe { ffi::EVP_PKEY_bits(self.as_ptr()) as u32 }
    }
}

impl<T> PKeyRef<T>
where
    T: HasPrivate,
{
    private_key_to_pem! {
        #[corresponds(PEM_write_bio_PKCS8PrivateKey)]
        private_key_to_pem_pkcs8,
        #[corresponds(PEM_write_bio_PKCS8PrivateKey)]
        private_key_to_pem_pkcs8_passphrase,
        ffi::PEM_write_bio_PKCS8PrivateKey
    }

    to_der! {
        #[corresponds(i2d_PrivateKey)]
        private_key_to_der,
        ffi::i2d_PrivateKey
    }

    private_key_to_pem! {
        #[corresponds(i2d_PKCS8PrivateKey_bio)]
        private_key_to_der_pkcs8,
        #[corresponds(i2d_PKCS8PrivateKey_bio)]
        private_key_to_der_pkcs8_passphrase,
        ffi::i2d_PKCS8PrivateKey_bio
    }
}

impl<T> fmt::Debug for PKey<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let alg = match self.id() {
            Id::RSA => "RSA",
            Id::RSAPSS => "RSAPSS",
            Id::DSA => "DSA",
            Id::DH => "DH",
            Id::EC => "EC",
            Id::ED25519 => "Ed25519",
            Id::ED448 => "Ed448",
            _ => "unknown",
        };
        fmt.debug_struct("PKey").field("algorithm", &alg).finish()
    }
}

impl<T> Clone for PKey<T> {
    fn clone(&self) -> PKey<T> {
        PKeyRef::to_owned(self)
    }
}

impl<T> PKey<T> {
    #[corresponds(EVP_PKEY_assign_RSA)]
    pub fn from_rsa(rsa: Rsa<T>) -> Result<PKey<T>, ErrorStack> {
        unsafe {
            let evp = cvt_p(ffi::EVP_PKEY_new())?;
            let pkey = PKey::from_ptr(evp);
            cvt(ffi::EVP_PKEY_assign(
                pkey.0,
                ffi::EVP_PKEY_RSA,
                rsa.as_ptr().cast(),
            ))?;
            mem::forget(rsa);
            Ok(pkey)
        }
    }

    #[corresponds(EVP_PKEY_assign_EC_KEY)]
    pub fn from_ec_key(ec_key: EcKey<T>) -> Result<PKey<T>, ErrorStack> {
        unsafe {
            let evp = cvt_p(ffi::EVP_PKEY_new())?;
            let pkey = PKey::from_ptr(evp);
            cvt(ffi::EVP_PKEY_assign(
                pkey.0,
                ffi::EVP_PKEY_EC,
                ec_key.as_ptr().cast(),
            ))?;
            mem::forget(ec_key);
            Ok(pkey)
        }
    }
}

impl PKey<Private> {
    #[corresponds(EVP_PKEY_keygen)]
    pub fn generate(id: Id) -> Result<PKey<Private>, ErrorStack> {
        unsafe {
            ffi::init();

            let ctx = cvt_p(ffi::EVP_PKEY_CTX_new_id(id.as_raw(), ptr::null_mut()))?;

            let result = (|| {
                cvt(ffi::EVP_PKEY_keygen_init(ctx))?;

                let mut pkey = ptr::null_mut();
                cvt(ffi::EVP_PKEY_keygen(ctx, &mut pkey))?;

                Ok(PKey::from_ptr(pkey))
            })();

            ffi::EVP_PKEY_CTX_free(ctx);
            result
        }
    }

    private_key_from_pem! {
        #[corresponds(PEM_read_bio_PrivateKey)]
        private_key_from_pem,

        #[corresponds(PEM_read_bio_PrivateKey)]
        private_key_from_pem_passphrase,

        #[corresponds(PEM_read_bio_PrivateKey)]
        private_key_from_pem_callback,
        PKey<Private>,
        ffi::PEM_read_bio_PrivateKey
    }

    from_der! {
        #[corresponds(d2i_AutoPrivateKey)]
        private_key_from_der,
        PKey<Private>,
        ffi::d2i_AutoPrivateKey,
        ::libc::c_long
    }
}

impl PKey<Public> {
    from_pem! {
        #[corresponds(PEM_read_bio_PUBKEY)]
        public_key_from_pem,
        PKey<Public>,
        ffi::PEM_read_bio_PUBKEY
    }

    from_der! {
        #[corresponds(d2i_PUBKEY)]
        public_key_from_der,
        PKey<Public>,
        ffi::d2i_PUBKEY,
        ::libc::c_long
    }
}

use crate::ffi::EVP_PKEY_up_ref;

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_private_key_from_pem() {
        let key = include_bytes!("../test/key.pem");
        PKey::private_key_from_pem(key).unwrap();
    }

    #[test]
    fn test_public_key_from_pem() {
        let key = include_bytes!("../test/key.pem.pub");
        PKey::public_key_from_pem(key).unwrap();
    }

    #[test]
    fn test_public_key_from_der() {
        let key = include_bytes!("../test/key.der.pub");
        PKey::public_key_from_der(key).unwrap();
    }

    #[test]
    fn test_private_key_from_der() {
        let key = include_bytes!("../test/key.der");
        PKey::private_key_from_der(key).unwrap();
    }

    #[test]
    fn test_pem() {
        let key = include_bytes!("../test/key.pem");
        let key = PKey::private_key_from_pem(key).unwrap();

        let priv_key = key.private_key_to_pem_pkcs8().unwrap();
        let pub_key = key.public_key_to_pem().unwrap();

        assert!(priv_key.windows(11).any(|s| s == b"PRIVATE KEY"));
        assert!(pub_key.windows(10).any(|s| s == b"PUBLIC KEY"));
    }

    #[test]
    fn test_der_pkcs8() {
        let key = include_bytes!("../test/key.der");
        let key = PKey::private_key_from_der(key).unwrap();

        let priv_key = key.private_key_to_der_pkcs8().unwrap();

        assert_eq!(hex::encode(&priv_key[4..=6]), "020100");
        assert_eq!(hex::encode(&priv_key[9..=19]), "06092a864886f70d010101");
    }

    #[test]
    fn test_generate_invalid_id() {
        assert!(PKey::generate(Id::from_raw(0)).is_err());
    }
}
