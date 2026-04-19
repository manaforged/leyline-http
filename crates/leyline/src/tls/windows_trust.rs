//! Windows system trust store bridge for BoringSSL.
//!
//! BoringSSL's `X509_STORE_set_default_paths()` (used by
//! `SslContextBuilder::set_default_verify_paths()`) points at Unix-style
//! locations such as `/etc/ssl/certs` that do not exist on Windows. Without
//! a bridge every HTTPS request fails at handshake with
//! `unable to get local issuer certificate`.
//!
//! This module reads the logical `"ROOT"` system certificate store
//! (which virtualises both `LocalMachine\Root` and `CurrentUser\Root`)
//! via the Win32 crypto API and returns each trusted root as DER bytes
//! for the caller to push into a BoringSSL `X509_STORE`.
//!
//! Intentionally does NOT depend on the `schannel` or
//! `rustls-native-certs` crates — both are banned in `deny.toml` because
//! they drag in alternative TLS backends. `windows-sys` is the raw Win32
//! binding and is not a TLS implementation.
//!
//! Safety: every `unsafe` block is confined to this file. All Win32
//! handles are freed on drop via the `RootStore` guard.

use std::ptr;

use windows_sys::Win32::Security::Cryptography::{
    CertCloseStore, CertEnumCertificatesInStore, CertOpenSystemStoreW, CERT_CONTEXT, HCERTSTORE,
};

/// RAII wrapper that closes the Win32 cert store on drop.
struct RootStore(HCERTSTORE);

impl Drop for RootStore {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from `CertOpenSystemStoreW` which returns
            // a valid store handle on success; we only construct `RootStore`
            // from such a handle. `CertCloseStore` accepts any valid handle
            // and the flags argument 0 means "release context references".
            unsafe {
                CertCloseStore(self.0, 0);
            }
        }
    }
}

/// Read every certificate in the Windows `"ROOT"` logical store and
/// return each as DER-encoded bytes.
///
/// Returns `Err(std::io::Error::last_os_error())` if the store cannot
/// be opened. An empty `Vec` is possible if the store is genuinely
/// empty (very rare in practice); the caller decides how to handle
/// zero roots.
pub(crate) fn load_system_roots() -> std::io::Result<Vec<Vec<u8>>> {
    // `"ROOT"` as a UTF-16 null-terminated string. Windows' crypto API
    // accepts either ASCII or wide; we use the wide variant to avoid
    // codepage surprises.
    let store_name: Vec<u16> = "ROOT\0".encode_utf16().collect();

    // SAFETY: `store_name` is a valid null-terminated UTF-16 string for
    // the duration of the call. `CertOpenSystemStoreW` takes ownership
    // of nothing and either returns a valid handle or a null pointer.
    let store = unsafe { CertOpenSystemStoreW(0, store_name.as_ptr()) };
    if store.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let store = RootStore(store);

    let mut out = Vec::new();
    let mut cert_ctx: *mut CERT_CONTEXT = ptr::null_mut();
    loop {
        // SAFETY: `store.0` is a valid open store (checked above).
        // `CertEnumCertificatesInStore` consumes the previous context
        // pointer (releasing it) and returns the next one, or null when
        // the enumeration is exhausted. Passing null on the first call
        // starts iteration from the beginning.
        cert_ctx = unsafe { CertEnumCertificatesInStore(store.0, cert_ctx) };
        if cert_ctx.is_null() {
            break;
        }

        // SAFETY: while non-null, `cert_ctx` points at a valid
        // `CERT_CONTEXT` owned by the enumeration. `pbCertEncoded` is a
        // pointer to `cbCertEncoded` bytes of the DER-encoded cert and
        // is valid until the next `CertEnumCertificatesInStore` call
        // (which frees the previous context for us). We copy the bytes
        // immediately so the resulting `Vec<u8>` outlives the context.
        let (ptr_encoded, len) = unsafe {
            let ctx = &*cert_ctx;
            (ctx.pbCertEncoded, ctx.cbCertEncoded as usize)
        };
        if ptr_encoded.is_null() || len == 0 {
            continue;
        }
        // SAFETY: `ptr_encoded` and `len` are the cert bytes described
        // above; the slice lives only for the copy into `der`.
        let der = unsafe { std::slice::from_raw_parts(ptr_encoded, len) }.to_vec();
        out.push(der);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: on a real Windows box the ROOT store is never empty —
    /// Microsoft ships with dozens of trusted roots by default. A zero
    /// count here means the bridge is broken.
    #[test]
    fn root_store_is_non_empty() {
        let roots = load_system_roots().expect("open ROOT store");
        assert!(
            !roots.is_empty(),
            "Windows ROOT store returned zero certificates — bridge is broken"
        );
        // Each DER cert starts with SEQUENCE tag 0x30.
        for (i, der) in roots.iter().enumerate() {
            assert_eq!(der.first(), Some(&0x30), "root #{i} is not DER-encoded");
        }
    }
}
