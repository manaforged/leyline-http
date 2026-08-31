//! macOS system trust store bridge for BoringSSL.

use std::collections::HashSet;
use std::ffi::c_void;

type CfArrayRef = *const c_void;
type CfDataRef = *const c_void;
type CfDictionaryRef = *const c_void;
type CfNumberRef = *const c_void;
type CfStringRef = *const c_void;
type CfTypeRef = *const c_void;
type CfIndex = isize;
type OsStatus = i32;
type SecCertificateRef = CfTypeRef;
type SecTrustSettingsDomain = i32;

const DOMAIN_USER: SecTrustSettingsDomain = 0;
const DOMAIN_ADMIN: SecTrustSettingsDomain = 1;
const DOMAIN_SYSTEM: SecTrustSettingsDomain = 2;

const RESULT_DENY: i64 = 1;
const RESULT_TRUST_ROOT: i64 = 2;
const RESULT_TRUST_AS_ROOT: i64 = 3;

const ERR_SEC_ITEM_NOT_FOUND: OsStatus = -25300;

const CF_NUMBER_SINT64: isize = 4;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// The `kSecTrustSettingsResult` dictionary key.
fn trust_settings_result_key() -> Option<OwnedCf> {
    // SAFETY: null allocator = kCFAllocatorDefault; input is a valid NUL-terminated UTF-8 literal; the returned string is owned.
    let key = unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            c"kSecTrustSettingsResult".as_ptr().cast(),
            CF_STRING_ENCODING_UTF8,
        )
    };
    OwnedCf::new(key)
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustCopyAnchorCertificates(anchors: *mut CfArrayRef) -> OsStatus;
    fn SecCertificateCopyData(certificate: SecCertificateRef) -> CfDataRef;
    fn SecTrustSettingsCopyCertificates(
        domain: SecTrustSettingsDomain,
        certArray: *mut CfArrayRef,
    ) -> OsStatus;
    fn SecTrustSettingsCopyTrustSettings(
        certificate: SecCertificateRef,
        domain: SecTrustSettingsDomain,
        trustSettings: *mut CfArrayRef,
    ) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CfArrayRef) -> CfIndex;
    fn CFArrayGetValueAtIndex(array: CfArrayRef, index: CfIndex) -> *const c_void;
    fn CFDataGetBytePtr(data: CfDataRef) -> *const u8;
    fn CFDataGetLength(data: CfDataRef) -> isize;
    fn CFDictionaryGetValue(dictionary: CfDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cStr: *const u8,
        encoding: u32,
    ) -> CfStringRef;
    fn CFNumberGetValue(number: CfNumberRef, theType: isize, valuePtr: *mut c_void) -> u8;
    fn CFGetTypeID(cf: CfTypeRef) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFRelease(value: *const c_void);
}

struct OwnedCf(*const c_void);

impl OwnedCf {
    fn new(value: *const c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }

    fn get(&self) -> *const c_void {
        self.0
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: every `OwnedCf` is created from a CoreFoundation Copy API.
        unsafe { CFRelease(self.0) };
    }
}

/// DER bytes of a `SecCertificateRef`.
unsafe fn certificate_der(cert: SecCertificateRef) -> Option<Vec<u8>> {
    // SAFETY: Copy API — the returned CFData is owned and released below.
    let data = OwnedCf::new(unsafe { SecCertificateCopyData(cert) })?;
    // SAFETY: `data` is live for this function's duration.
    let length = unsafe { CFDataGetLength(data.get()) };
    // SAFETY: `data` is live.
    let bytes = unsafe { CFDataGetBytePtr(data.get()) };
    if length <= 0 || bytes.is_null() {
        return None;
    }
    // SAFETY: CoreFoundation guarantees `length` readable bytes at `bytes`.
    Some(unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec())
}

/// Collect `(denied, trusted)` DER sets from one trust-settings domain.
fn domain_trust_sets(domain: SecTrustSettingsDomain) -> (HashSet<Vec<u8>>, HashSet<Vec<u8>>) {
    let mut denied = HashSet::new();
    let mut trusted = HashSet::new();

    let Some(result_key) = trust_settings_result_key() else {
        return (denied, trusted);
    };

    let mut certs = std::ptr::null();
    // SAFETY: out-pointer is valid; the returned array is owned (released via `OwnedCf`) and valid for this function's duration.
    let status = unsafe { SecTrustSettingsCopyCertificates(domain, &mut certs) };
    if status != 0 {
        if status != ERR_SEC_ITEM_NOT_FOUND {
            tracing::debug!(
                target: "leyline::tls::trust",
                domain,
                status,
                "SecTrustSettingsCopyCertificates failed; domain skipped"
            );
        }
        return (denied, trusted);
    }
    let Some(certs) = OwnedCf::new(certs) else {
        return (denied, trusted);
    };
    // SAFETY: `certs` is a live CFArray for this function's duration.
    let count = unsafe { CFArrayGetCount(certs.get()) };
    for index in 0..count.max(0) {
        // SAFETY: index is within the CFArray bounds.
        let cert = unsafe { CFArrayGetValueAtIndex(certs.get(), index) };
        if cert.is_null() {
            continue;
        }
        // SAFETY: `cert` is a live certificate reference from the array; the returned Vec owns its byte copy.
        let Some(der) = (unsafe { certificate_der(cert) }) else {
            continue;
        };

        let mut settings = std::ptr::null();
        // SAFETY: `cert` is valid; the returned settings array is owned (released via `OwnedCf`) and valid for this function's duration.
        let status = unsafe { SecTrustSettingsCopyTrustSettings(cert, domain, &mut settings) };
        if status != 0 {
            if status != ERR_SEC_ITEM_NOT_FOUND {
                tracing::debug!(
                    target: "leyline::tls::trust",
                    domain,
                    status,
                    "SecTrustSettingsCopyTrustSettings failed; certificate skipped"
                );
            }
            continue;
        }
        let Some(settings) = OwnedCf::new(settings) else {
            continue;
        };
        // SAFETY: `settings` is live.
        let entries = unsafe { CFArrayGetCount(settings.get()) };
        let mut grant = false;
        let mut deny = false;
        for entry in 0..entries.max(0) {
            // SAFETY: index is within the CFArray bounds.
            let dict = unsafe { CFArrayGetValueAtIndex(settings.get(), entry) };
            if dict.is_null() {
                continue;
            }
            // SAFETY: `dict` is a live CFDictionary; the key is a value- equal CFString for kSecTrustSettingsResult.
            let raw = unsafe { CFDictionaryGetValue(dict, result_key.get()) };
            if raw.is_null() {
                grant = true;
                continue;
            }
            // SAFETY: `raw` is a live CFNumber (or ignored below if not).
            let is_number = unsafe { CFGetTypeID(raw) == CFNumberGetTypeID() };
            if !is_number {
                continue;
            }
            let mut result: i64 = 0;
            // SAFETY: `raw` is a live CFNumber and `result` outlives the call.
            unsafe {
                CFNumberGetValue(
                    raw,
                    CF_NUMBER_SINT64,
                    &mut result as *mut i64 as *mut c_void,
                )
            };
            match result {
                RESULT_DENY => deny = true,
                RESULT_TRUST_ROOT | RESULT_TRUST_AS_ROOT => grant = true,
                _ => {}
            }
        }
        if deny {
            denied.insert(der);
        } else if grant {
            trusted.insert(der);
        }
    }
    (denied, trusted)
}

/// Return the effective system trust set as DER: built-in anchors minus every domain's explicit deny, plus every trust-settings root.
pub(crate) fn load_system_roots() -> std::io::Result<Vec<Vec<u8>>> {
    let mut denied_all: HashSet<Vec<u8>> = HashSet::new();
    let mut trusted_all: HashSet<Vec<u8>> = HashSet::new();
    for domain in [DOMAIN_USER, DOMAIN_ADMIN, DOMAIN_SYSTEM] {
        let (denied, trusted) = domain_trust_sets(domain);
        denied_all.extend(denied);
        trusted_all.extend(trusted);
    }

    let mut anchors = std::ptr::null();
    // SAFETY: `anchors` is a valid out-pointer and the returned array is owned.
    let status = unsafe { SecTrustCopyAnchorCertificates(&mut anchors) };
    if status != 0 {
        return Err(std::io::Error::other(format!(
            "SecTrustCopyAnchorCertificates failed with OSStatus {status}"
        )));
    }
    let Some(anchors) = OwnedCf::new(anchors) else {
        return Err(std::io::Error::other(
            "macOS returned a null trust anchor array",
        ));
    };
    // SAFETY: `anchors` owns a live CFArray for this function's duration.
    let count = unsafe { CFArrayGetCount(anchors.get()) };
    if count < 0 {
        return Err(std::io::Error::other(
            "macOS returned a negative trust anchor count",
        ));
    }

    let mut roots: HashSet<Vec<u8>> = HashSet::new();
    for index in 0..count {
        // SAFETY: index is within the CFArray bounds established above.
        let certificate = unsafe { CFArrayGetValueAtIndex(anchors.get(), index) };
        if certificate.is_null() {
            continue;
        }
        // SAFETY: `certificate` is a live certificate reference from the anchor array; the returned Vec owns its byte copy.
        let Some(der) = (unsafe { certificate_der(certificate) }) else {
            continue;
        };
        if !denied_all.contains(&der) {
            roots.insert(der);
        }
    }
    for der in trusted_all {
        if !denied_all.contains(&der) {
            roots.insert(der);
        }
    }

    Ok(roots.into_iter().collect())
}

#[cfg(test)]
mod tests;
