//! macOS system trust store bridge for BoringSSL.

use std::ffi::c_void;

type CfArrayRef = *const c_void;
type CfDataRef = *const c_void;
type SecCertificateRef = *const c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustCopyAnchorCertificates(anchors: *mut CfArrayRef) -> i32;
    fn SecCertificateCopyData(certificate: SecCertificateRef) -> CfDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CfArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CfArrayRef, index: isize) -> *const c_void;
    fn CFDataGetBytePtr(data: CfDataRef) -> *const u8;
    fn CFDataGetLength(data: CfDataRef) -> isize;
    fn CFRelease(value: *const c_void);
}

struct OwnedCf(*const c_void);

impl OwnedCf {
    fn new(value: *const c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: every `OwnedCf` is created from a CoreFoundation Copy API.
        unsafe { CFRelease(self.0) };
    }
}

/// Return every anchor from macOS's actual system root keychain as DER.
pub(crate) fn load_system_roots() -> std::io::Result<Vec<Vec<u8>>> {
    let mut anchors = std::ptr::null();
    // SAFETY: `anchors` is a valid out-pointer and the returned array is owned.
    let status = unsafe { SecTrustCopyAnchorCertificates(&mut anchors) };
    if status != 0 {
        return Err(std::io::Error::other(format!(
            "SecTrustCopyAnchorCertificates failed with OSStatus {status}"
        )));
    }
    let anchors = OwnedCf::new(anchors)
        .ok_or_else(|| std::io::Error::other("macOS returned a null trust anchor array"))?;
    // SAFETY: `anchors` owns a live CFArray for this function's duration.
    let count = unsafe { CFArrayGetCount(anchors.0) };
    if count < 0 {
        return Err(std::io::Error::other(
            "macOS returned a negative trust anchor count",
        ));
    }

    let mut roots = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index` is within the CFArray bounds established above.
        let certificate = unsafe { CFArrayGetValueAtIndex(anchors.0, index) };
        if certificate.is_null() {
            continue;
        }
        // SAFETY: the array value is a SecCertificateRef supplied by Security.framework.
        let Some(data) = OwnedCf::new(unsafe { SecCertificateCopyData(certificate) }) else {
            continue;
        };
        // SAFETY: `data` owns a live CFData object.
        let length = unsafe { CFDataGetLength(data.0) };
        // SAFETY: `data` remains live until after the bytes are copied below.
        let bytes = unsafe { CFDataGetBytePtr(data.0) };
        if length > 0 && !bytes.is_null() {
            // SAFETY: CoreFoundation guarantees `length` readable bytes at `bytes`.
            roots.push(unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec());
        }
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_store_is_non_empty_der() {
        let roots = load_system_roots().expect("read macOS trust anchors");
        assert!(!roots.is_empty(), "macOS system trust store is empty");
        for (index, der) in roots.iter().enumerate() {
            assert_eq!(der.first(), Some(&0x30), "root #{index} is not DER");
        }
    }
}
