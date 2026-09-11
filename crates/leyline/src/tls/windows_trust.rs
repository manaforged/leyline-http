use std::ptr;

use windows_sys::Win32::Security::Cryptography::{
    CERT_CONTEXT, CertCloseStore, CertEnumCertificatesInStore, CertOpenSystemStoreW, HCERTSTORE,
};

struct RootStore(HCERTSTORE);

impl Drop for RootStore {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` came from `CertOpenSystemStoreW` which returns a valid store handle on success; we only construct `RootStore` from such a handle. `CertCloseStore` accepts any valid handle and the flags argument 0 means "release context references".
            unsafe {
                CertCloseStore(self.0, 0);
            }
        }
    }
}

pub(crate) fn load_system_roots() -> std::io::Result<Vec<Vec<u8>>> {
    let store_name: Vec<u16> = "ROOT\0".encode_utf16().collect();

    // SAFETY: `store_name` is a valid null-terminated UTF-16 string for the duration of the call. `CertOpenSystemStoreW` takes ownership of nothing and either returns a valid handle or a null pointer.
    let store = unsafe { CertOpenSystemStoreW(0, store_name.as_ptr()) };
    if store.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let store = RootStore(store);

    let mut out = Vec::new();
    let mut cert_ctx: *mut CERT_CONTEXT = ptr::null_mut();
    loop {
        // SAFETY: `store.0` is a valid open store (checked above). `CertEnumCertificatesInStore` consumes the previous context pointer (releasing it) and returns the next one, or null when the enumeration is exhausted. Passing null on the first call starts iteration from the beginning.
        cert_ctx = unsafe { CertEnumCertificatesInStore(store.0, cert_ctx) };
        if cert_ctx.is_null() {
            break;
        }

        // SAFETY: while non-null, `cert_ctx` points at a valid `CERT_CONTEXT` owned by the enumeration. `pbCertEncoded` is a pointer to `cbCertEncoded` bytes of the DER-encoded cert and is valid until the next `CertEnumCertificatesInStore` call (which frees the previous context for us). We copy the bytes immediately so the resulting `Vec<u8>` outlives the context.
        let (ptr_encoded, len) = unsafe {
            let ctx = &*cert_ctx;
            (ctx.pbCertEncoded, ctx.cbCertEncoded as usize)
        };
        if ptr_encoded.is_null() || len == 0 {
            continue;
        }
        // SAFETY: `ptr_encoded` and `len` are the cert bytes described above; the slice lives only for the copy into `der`.
        let der = unsafe { std::slice::from_raw_parts(ptr_encoded, len) }.to_vec();
        out.push(der);
    }

    Ok(out)
}

#[cfg(test)]
mod tests;
