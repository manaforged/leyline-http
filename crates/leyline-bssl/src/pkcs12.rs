use std::ffi::CString;
use std::fmt;
use std::ptr;
use std::slice;

use foreign_types::{ForeignType, ForeignTypeRef};

use crate::error::ErrorStack;
use crate::pkey::{PKey, PKeyRef, Private};
use crate::stack::{Stack, StackRef};
use crate::x509::{X509Ref, X509};
use crate::{cvt, cvt_p, ffi};

/// Parsed contents of a DER-encoded PKCS#12 archive.
pub struct ParsedPkcs12 {
    pub pkey: PKey<Private>,
    pub cert: X509,
    pub chain: Vec<X509>,
}

impl fmt::Debug for ParsedPkcs12 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedPkcs12")
            .field("chain_len", &self.chain.len())
            .finish_non_exhaustive()
    }
}

struct RawPkcs12(*mut ffi::PKCS12);

impl Drop for RawPkcs12 {
    fn drop(&mut self) {
        // SAFETY: self.0 was returned by d2i_PKCS12 or PKCS12_create and is non-null.
        unsafe { ffi::PKCS12_free(self.0) };
    }
}

/// Parses a DER-encoded PKCS#12 archive using `password`.
pub fn parse(der: &[u8], password: &str) -> Result<ParsedPkcs12, ErrorStack> {
    let pass = CString::new(password)
        .map_err(|_| ErrorStack::internal_error_str("pkcs12 password contains interior NUL"))?;
    let mut input = der.as_ptr();
    // SAFETY: input points to der.len() readable bytes; d2i_PKCS12 returns an owned PKCS12 or null.
    let raw = RawPkcs12(unsafe { cvt_p(ffi::d2i_PKCS12(ptr::null_mut(), &mut input, der.len()))? });

    let mut pkey_ptr: *mut ffi::EVP_PKEY = ptr::null_mut();
    let mut cert_ptr: *mut ffi::X509 = ptr::null_mut();
    let mut ca_ptr: *mut ffi::stack_st_X509 = ptr::null_mut();
    // SAFETY: raw.0 is valid, pass is NUL-terminated, and output pointers are valid stack locals.
    unsafe {
        cvt(ffi::PKCS12_parse(
            raw.0,
            pass.as_ptr(),
            &mut pkey_ptr,
            &mut cert_ptr,
            &mut ca_ptr,
        ))?;
    }

    // SAFETY: non-null outputs returned by PKCS12_parse are owned by the caller.
    let pkey = (!pkey_ptr.is_null()).then(|| unsafe { PKey::from_ptr(pkey_ptr) });
    let cert = (!cert_ptr.is_null()).then(|| unsafe { X509::from_ptr(cert_ptr) });
    let chain = if ca_ptr.is_null() {
        Vec::new()
    } else {
        // SAFETY: ca_ptr is a caller-owned STACK_OF(X509) returned by PKCS12_parse.
        unsafe { Stack::<X509>::from_ptr(ca_ptr) }
            .into_iter()
            .collect()
    };

    let pkey = pkey.ok_or_else(|| ErrorStack::internal_error_str("pkcs12 archive has no key"))?;
    let cert =
        cert.ok_or_else(|| ErrorStack::internal_error_str("pkcs12 archive has no certificate"))?;
    Ok(ParsedPkcs12 { pkey, cert, chain })
}

/// Builds a DER-encoded PKCS#12 archive from a private key, leaf certificate, and optional chain.
pub fn create(
    password: &str,
    name: &str,
    pkey: &PKeyRef<Private>,
    cert: &X509Ref,
    chain: Option<&StackRef<X509>>,
) -> Result<Vec<u8>, ErrorStack> {
    let pass = CString::new(password)
        .map_err(|_| ErrorStack::internal_error_str("pkcs12 password contains interior NUL"))?;
    let friendly = CString::new(name)
        .map_err(|_| ErrorStack::internal_error_str("pkcs12 name contains interior NUL"))?;
    let chain_ptr = chain.map_or(ptr::null(), |c| c.as_ptr().cast_const());
    // SAFETY: all pointers are valid for the duration of PKCS12_create.
    let raw = RawPkcs12(unsafe {
        cvt_p(ffi::PKCS12_create(
            pass.as_ptr(),
            friendly.as_ptr(),
            pkey.as_ptr(),
            cert.as_ptr(),
            chain_ptr,
            0,
            0,
            0,
            0,
            0,
        ))?
    });

    let mut out: *mut u8 = ptr::null_mut();
    // SAFETY: i2d_PKCS12 allocates a buffer with OPENSSL_malloc when *out is null and returns its length.
    let len = unsafe { ffi::i2d_PKCS12(raw.0, &mut out) };
    if len < 0 || out.is_null() {
        if !out.is_null() {
            // SAFETY: out was allocated by BoringSSL.
            unsafe { ffi::OPENSSL_free(out.cast()) };
        }
        return Err(ErrorStack::get());
    }
    // SAFETY: out points to len initialized bytes allocated by BoringSSL and freed immediately after copy.
    let bytes = unsafe {
        let copy = slice::from_raw_parts(out, len as usize).to_vec();
        ffi::OPENSSL_free(out.cast());
        copy
    };
    Ok(bytes)
}
