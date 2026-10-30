//! A collection of numerical identifiers for OpenSSL objects.
use crate::ffi;
use libc::c_int;
use openssl_macros::corresponds;

/// The digest and public-key algorithms associated with a signature.
pub struct SignatureAlgorithms {
    /// The signature's digest.
    ///
    /// If the signature does not specify a digest, this will be `NID::UNDEF`.
    pub digest: Nid,

    /// The signature's public-key.
    pub pkey: Nid,
}

/// A numerical identifier for an OpenSSL object.
///
/// Objects in OpenSSL can have a short name, a long name, and
/// a numerical identifier (NID). For convenience, objects
/// are usually represented in source code using these numeric
/// identifiers.
///
/// Users should generally not need to create new `Nid`s.
///
/// # External Documentation
///
/// The following documentation provides context about `Nid`s and their usage
/// in OpenSSL.
///
/// - [Obj_nid2obj](https://www.openssl.org/docs/man1.1.0/crypto/OBJ_create.html)
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Nid(c_int);

#[allow(non_snake_case)]
impl Nid {
    /// Create a `Nid` from an integer representation.
    #[must_use]
    pub fn from_raw(raw: c_int) -> Nid {
        Nid(raw)
    }

    /// Return the integer representation of a `Nid`.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }

    /// Returns the `Nid`s of the digest and public key algorithms associated with a signature ID.
    #[corresponds(OBJ_find_sigid_algs)]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn signature_algorithms(&self) -> Option<SignatureAlgorithms> {
        unsafe {
            let mut digest = 0;
            let mut pkey = 0;
            if ffi::OBJ_find_sigid_algs(self.0, &mut digest, &mut pkey) == 1 {
                Some(SignatureAlgorithms {
                    digest: Nid(digest),
                    pkey: Nid(pkey),
                })
            } else {
                None
            }
        }
    }

    pub const X9_62_PRIME256V1: Nid = Nid(ffi::NID_X9_62_prime256v1);
    pub const SECP384R1: Nid = Nid(ffi::NID_secp384r1);
    pub const SECP521R1: Nid = Nid(ffi::NID_secp521r1);
    pub const MD5: Nid = Nid(ffi::NID_md5);
    pub const ALGORITHM: Nid = Nid(ffi::NID_algorithm);
    pub const SHA: Nid = Nid(ffi::NID_sha);
    pub const X509: Nid = Nid(ffi::NID_X509);
    pub const TITLE: Nid = Nid(ffi::NID_title);
    pub const OWNER: Nid = Nid(ffi::NID_owner);
    pub const SUBJECT_KEY_IDENTIFIER: Nid = Nid(ffi::NID_subject_key_identifier);
    pub const KEY_USAGE: Nid = Nid(ffi::NID_key_usage);
    pub const SUBJECT_ALT_NAME: Nid = Nid(ffi::NID_subject_alt_name);
    pub const BASIC_CONSTRAINTS: Nid = Nid(ffi::NID_basic_constraints);
    pub const AUTHORITY_KEY_IDENTIFIER: Nid = Nid(ffi::NID_authority_key_identifier);
    pub const EXT_KEY_USAGE: Nid = Nid(ffi::NID_ext_key_usage);
    pub const IANA: Nid = Nid(ffi::NID_iana);
    pub const DATA: Nid = Nid(ffi::NID_data);
    pub const HOST: Nid = Nid(ffi::NID_host);
}

#[cfg(test)]
mod test {}
