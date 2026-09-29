use leyline_bssl::ssl::SslRef;
use std::ffi::c_void;
use std::io::{Error, Result};
use std::ptr::null;

type CfRef = *const c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCertificateCreateWithData(allocator: CfRef, data: CfRef) -> CfRef;
    fn SecPolicyCreateSSL(server: u8, hostname: CfRef) -> CfRef;
    fn SecTrustCreateWithCertificates(
        certificates: CfRef,
        policies: CfRef,
        trust: *mut CfRef,
    ) -> i32;
    fn SecTrustSetNetworkFetchAllowed(trust: CfRef, allowed: u8) -> i32;
    fn SecTrustEvaluateWithError(trust: CfRef, error: *mut CfRef) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayCreate(
        allocator: CfRef,
        values: *const CfRef,
        count: isize,
        callbacks: CfRef,
    ) -> CfRef;
    fn CFDataCreate(allocator: CfRef, bytes: *const u8, length: isize) -> CfRef;
    fn CFStringCreateWithBytes(
        allocator: CfRef,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> CfRef;
    fn CFRelease(value: CfRef);
}

struct OwnedCf(CfRef);

impl OwnedCf {
    fn new(value: CfRef) -> Result<Self> {
        if value.is_null() {
            Err(Error::other(
                "macOS returned a null certificate evaluation object",
            ))
        } else {
            Ok(Self(value))
        }
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: this object owns a non-null reference returned by a CoreFoundation Create API.
        unsafe { CFRelease(self.0) };
    }
}

fn certificate(der: &[u8]) -> Result<OwnedCf> {
    let length = isize::try_from(der.len()).map_err(Error::other)?;
    // SAFETY: the byte slice is valid for length bytes and CFDataCreate copies it.
    let data = OwnedCf::new(unsafe { CFDataCreate(null(), der.as_ptr(), length) })?;
    // SAFETY: data owns a live CFData; the created certificate owns its backing data.
    OwnedCf::new(unsafe { SecCertificateCreateWithData(null(), data.0) })
}

fn array(values: &[OwnedCf]) -> Result<OwnedCf> {
    let pointers: Vec<CfRef> = values.iter().map(|value| value.0).collect();
    let count = isize::try_from(pointers.len()).map_err(Error::other)?;
    // SAFETY: CFArrayCreate copies the pointers; callers retain the objects until after the array and its trust evaluation are dropped.
    OwnedCf::new(unsafe { CFArrayCreate(null(), pointers.as_ptr(), count, null()) })
}

fn check(status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::other(format!(
            "macOS certificate evaluation setup failed: OSStatus {status}"
        )))
    }
}

fn evaluate(chain: &[Vec<u8>], host: &str) -> Result<bool> {
    if chain.is_empty() || host.is_empty() {
        return Err(Error::other(
            "certificate evaluation needs a peer chain and hostname",
        ));
    }
    let certificates = chain
        .iter()
        .map(|der| certificate(der))
        .collect::<Result<Vec<_>>>()?;
    let certificates_array = array(&certificates)?;
    let length = isize::try_from(host.len()).map_err(Error::other)?;
    // SAFETY: the host bytes are valid UTF-8, copied by the Create API; 0x08000100 is kCFStringEncodingUTF8.
    let hostname = OwnedCf::new(unsafe {
        CFStringCreateWithBytes(null(), host.as_ptr(), length, 0x0800_0100, 0)
    })?;
    // SAFETY: hostname is a live CFString; true selects verification of a TLS server certificate.
    let policy = OwnedCf::new(unsafe { SecPolicyCreateSSL(1, hostname.0) })?;
    let mut raw = null();
    // SAFETY: both input references are live, and raw is a valid output pointer.
    check(unsafe { SecTrustCreateWithCertificates(certificates_array.0, policy.0, &mut raw) })?;
    let trust = OwnedCf::new(raw)?;
    // SAFETY: trust is live; no platform network fetch may bypass the caller's transport or proxy.
    check(unsafe { SecTrustSetNetworkFetchAllowed(trust.0, 0) })?;
    // SAFETY: trust and all its inputs remain live until evaluation returns; a null error output is supported.
    Ok(unsafe { SecTrustEvaluateWithError(trust.0, std::ptr::null_mut()) })
}

pub(crate) fn verify(ssl: &SslRef, host: &str) -> Result<bool> {
    let leaf = ssl
        .peer_certificate()
        .ok_or_else(|| Error::other("peer certificate missing"))?;
    let chain = ssl
        .peer_cert_chain()
        .ok_or_else(|| Error::other("peer certificate chain missing"))?;
    let mut certificates = vec![leaf.to_der().map_err(Error::other)?];
    for certificate in chain {
        let der = certificate.to_der().map_err(Error::other)?;
        if der != certificates[0] {
            certificates.push(der);
        }
    }
    evaluate(&certificates, host)
}

#[cfg(test)]
mod tests;
