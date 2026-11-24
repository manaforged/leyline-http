#![forbid(unsafe_op_in_unsafe_fn)]

use super::{
    AlpnError, CertificateCompressor, ClientHello, GetSessionPendingError, PrivateKeyMethod,
    PrivateKeyMethodError, SelectCertError, Ssl, SslContext, SslRef, SslSession,
    SslSignatureAlgorithm, SslVerifyError, SESSION_CTX_INDEX,
};
use crate::error::ErrorStack;
use crate::ffi;
use crate::x509::{X509StoreContext, X509StoreContextRef};
use foreign_types::ForeignType;
use foreign_types::ForeignTypeRef;
use libc::{c_char, c_int, c_uchar, c_uint, c_void};
use std::ffi::CStr;
use std::mem::{self, MaybeUninit};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;
use std::str;
use std::sync::Arc;

fn catch_ffi<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

unsafe fn c_slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 || ptr.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees `ptr` points to `len` readable initialized bytes.
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

unsafe fn c_slice_mut<'a>(ptr: *mut u8, len: usize) -> &'a mut [u8] {
    if len == 0 || ptr.is_null() {
        &mut []
    } else {
        // SAFETY: caller guarantees `ptr` points to `len` uniquely accessible initialized bytes.
        unsafe { slice::from_raw_parts_mut(ptr, len) }
    }
}

pub extern "C" fn raw_verify<F>(preverify_ok: c_int, x509_ctx: *mut ffi::X509_STORE_CTX) -> c_int
where
    F: Fn(bool, &mut X509StoreContextRef) -> bool + 'static + Sync + Send,
{
    catch_ffi(0, || {
        if x509_ctx.is_null() {
            return 0;
        }
        // SAFETY: boring provides a live X509_STORE_CTX for the duration of this callback.
        let ctx = unsafe { X509StoreContextRef::from_ptr_mut(x509_ctx) };
        let Ok(ssl_idx) = X509StoreContext::ssl_idx() else {
            return 0;
        };
        let verify_idx = SslContext::cached_ex_index::<F>();
        let Some(ssl_context) = ctx.ex_data(ssl_idx).map(|ssl| ssl.ssl_context().to_owned()) else {
            return 0;
        };
        let Some(verify) = ssl_context.ex_data(verify_idx) else {
            return 0;
        };
        c_int::from(verify(preverify_ok != 0, ctx))
    })
}

pub(super) unsafe extern "C" fn raw_custom_verify<F>(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
) -> ffi::ssl_verify_result_t
where
    F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
{
    catch_ffi(ffi::ssl_verify_result_t::ssl_verify_invalid, || {
        let callback = |ssl: &mut SslRef| {
            let custom_verify_idx = SslContext::cached_ex_index::<F>();
            let ssl_context = ssl.ssl_context().to_owned();
            let Some(callback) = ssl_context.ex_data(custom_verify_idx) else {
                return Err(SslVerifyError::Invalid(super::SslAlert::INTERNAL_ERROR));
            };
            callback(ssl)
        };
        // SAFETY: boring provides live `ssl` and `out_alert` pointers for this callback.
        unsafe { raw_custom_verify_callback(ssl, out_alert, callback) }
    })
}

pub(super) unsafe extern "C" fn ssl_raw_custom_verify<F>(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
) -> ffi::ssl_verify_result_t
where
    F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
{
    catch_ffi(ffi::ssl_verify_result_t::ssl_verify_invalid, || {
        let callback = |ssl: &mut SslRef| {
            let Some(callback) = ssl.ex_data(Ssl::cached_ex_index::<Arc<F>>()).cloned() else {
                return Err(SslVerifyError::Invalid(super::SslAlert::INTERNAL_ERROR));
            };
            callback(ssl)
        };
        // SAFETY: boring provides live `ssl` and `out_alert` pointers for this callback.
        unsafe { raw_custom_verify_callback(ssl, out_alert, callback) }
    })
}

unsafe fn raw_custom_verify_callback(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
    callback: impl FnOnce(&mut SslRef) -> Result<(), SslVerifyError>,
) -> ffi::ssl_verify_result_t {
    if ssl.is_null() || out_alert.is_null() {
        return ffi::ssl_verify_result_t::ssl_verify_invalid;
    }
    // SAFETY: boring provides valid non-null inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    // SAFETY: boring provides a valid writable `out_alert` pointer.
    let out_alert = unsafe { &mut *out_alert };

    match callback(ssl) {
        Ok(()) => ffi::ssl_verify_result_t::ssl_verify_ok,
        Err(SslVerifyError::Invalid(alert)) => {
            *out_alert = alert.0 as u8;

            ffi::ssl_verify_result_t::ssl_verify_invalid
        }
        Err(SslVerifyError::Retry) => ffi::ssl_verify_result_t::ssl_verify_retry,
    }
}

pub(super) unsafe extern "C" fn ssl_raw_verify<F>(
    preverify_ok: c_int,
    x509_ctx: *mut ffi::X509_STORE_CTX,
) -> c_int
where
    F: Fn(bool, &mut X509StoreContextRef) -> bool + 'static + Sync + Send,
{
    catch_ffi(0, || {
        if x509_ctx.is_null() {
            return 0;
        }
        // SAFETY: boring provides valid inputs.
        let ctx = unsafe { X509StoreContextRef::from_ptr_mut(x509_ctx) };
        let Ok(ssl_idx) = X509StoreContext::ssl_idx() else {
            return 0;
        };
        let Some(callback) = ctx
            .ex_data(ssl_idx)
            .and_then(|ssl| ssl.ex_data(Ssl::cached_ex_index::<Arc<F>>()))
            .cloned()
        else {
            return 0;
        };
        c_int::from(callback(preverify_ok != 0, ctx))
    })
}

pub(super) unsafe extern "C" fn raw_alpn_select<F>(
    ssl: *mut ffi::SSL,
    out: *mut *const c_uchar,
    outlen: *mut c_uchar,
    inbuf: *const c_uchar,
    inlen: c_uint,
    _arg: *mut c_void,
) -> c_int
where
    F: for<'a> Fn(&mut SslRef, &'a [u8]) -> Result<&'a [u8], AlpnError> + 'static + Sync + Send,
{
    catch_ffi(ffi::SSL_TLSEXT_ERR_NOACK, || {
        if ssl.is_null() || out.is_null() || outlen.is_null() {
            return ffi::SSL_TLSEXT_ERR_NOACK;
        }
        // SAFETY: boring provides valid non-null inputs.
        let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
        // SAFETY: boring passes `inlen` readable bytes at `inbuf` when `inlen > 0`.
        let protos = unsafe { c_slice(inbuf, inlen as usize) };
        // SAFETY: boring provides valid writable `out` and `outlen` pointers.
        let out = unsafe { &mut *out };
        // SAFETY: boring provides a valid writable `outlen` pointer.
        let outlen = unsafe { &mut *outlen };

        let ssl_context = ssl.ssl_context().to_owned();
        let Some(callback) = ssl_context.ex_data(SslContext::cached_ex_index::<F>()) else {
            return ffi::SSL_TLSEXT_ERR_NOACK;
        };

        match callback(ssl, protos) {
            Ok(proto) => {
                let Ok(len) = c_uchar::try_from(proto.len()) else {
                    return ffi::SSL_TLSEXT_ERR_NOACK;
                };
                *out = proto.as_ptr();
                *outlen = len;
                ffi::SSL_TLSEXT_ERR_OK
            }
            Err(e) => e.0,
        }
    })
}

pub(super) unsafe extern "C" fn raw_select_cert<F>(
    client_hello: *const ffi::SSL_CLIENT_HELLO,
) -> ffi::ssl_select_cert_result_t
where
    F: Fn(ClientHello<'_>) -> Result<(), SelectCertError> + Sync + Send + 'static,
{
    catch_ffi(ffi::ssl_select_cert_result_t::ssl_select_cert_error, || {
        if client_hello.is_null() {
            return ffi::ssl_select_cert_result_t::ssl_select_cert_error;
        }
        // SAFETY: boring provides a valid `SSL_CLIENT_HELLO` pointer for this callback.
        let client_hello = ClientHello(unsafe { &*client_hello });

        let ssl_context = client_hello.ssl().ssl_context().to_owned();
        let Some(callback) = ssl_context.ex_data(SslContext::cached_ex_index::<F>()) else {
            return ffi::ssl_select_cert_result_t::ssl_select_cert_error;
        };

        match callback(client_hello) {
            Ok(()) => ffi::ssl_select_cert_result_t::ssl_select_cert_success,
            Err(e) => e.0,
        }
    })
}

pub(super) unsafe extern "C" fn raw_new_session<F>(
    ssl: *mut ffi::SSL,
    session: *mut ffi::SSL_SESSION,
) -> c_int
where
    F: Fn(&mut SslRef, SslSession) + 'static + Sync + Send,
{
    catch_ffi(0, || {
        if ssl.is_null() || session.is_null() {
            return 0;
        }
        // SAFETY: boring provides valid live `ssl` and caller-owned `session`.
        let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
        // SAFETY: returning 1 transfers ownership of `session` to Rust; on early 0 return we must not drop it.
        let Some(session_ctx) = ssl.ex_data(*SESSION_CTX_INDEX).cloned() else {
            return 0;
        };
        let Some(callback) = session_ctx.ex_data(SslContext::cached_ex_index::<F>()) else {
            return 0;
        };
        // SAFETY: returning 1 below tells BoringSSL that this callback took ownership of `session`.
        let session = unsafe { SslSession::from_ptr(session) };
        callback(ssl, session);
        1
    })
}

type DataPtr = *const c_uchar;

pub(super) unsafe extern "C" fn raw_get_session<F>(
    ssl: *mut ffi::SSL,
    data: DataPtr,
    len: c_int,
    copy: *mut c_int,
) -> *mut ffi::SSL_SESSION
where
    F: Fn(&mut SslRef, &[u8]) -> Result<Option<SslSession>, GetSessionPendingError>
        + 'static
        + Sync
        + Send,
{
    catch_ffi(ptr::null_mut(), || {
        if ssl.is_null() || copy.is_null() || len < 0 {
            return ptr::null_mut();
        }
        // SAFETY: boring provides valid non-null inputs.
        let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
        // SAFETY: `len >= 0` is checked above; `c_slice` handles zero length or null safely.
        let data = unsafe { c_slice(data, len as usize) };
        // SAFETY: `copy` is checked non-null above.
        let copy = unsafe { &mut *copy };

        let Some(session_ctx) = ssl.ex_data(*SESSION_CTX_INDEX).cloned() else {
            return ptr::null_mut();
        };
        let Some(callback) = session_ctx.ex_data(SslContext::cached_ex_index::<F>()) else {
            return ptr::null_mut();
        };

        match callback(ssl, data) {
            Ok(Some(session)) => {
                let p = session.into_ptr();
                *copy = 0;
                p
            }
            Ok(None) => ptr::null_mut(),
            // SAFETY: BoringSSL magic sentinel pointer indicating async session lookup pending.
            Err(GetSessionPendingError) => unsafe { ffi::SSL_magic_pending_session_ptr() },
        }
    })
}

pub(super) unsafe extern "C" fn raw_keylog<F>(ssl: *const ffi::SSL, line: *const c_char)
where
    F: Fn(&SslRef, &str) + 'static + Sync + Send,
{
    catch_ffi((), || {
        if ssl.is_null() || line.is_null() {
            return;
        }
        // SAFETY: boring provides valid non-null `ssl` and NUL-terminated `line`.
        let ssl = unsafe { SslRef::from_ptr(ssl.cast_mut()) };
        // SAFETY: boring passes a valid NUL-terminated keylog line.
        let line = unsafe { CStr::from_ptr(line).to_string_lossy() };

        let ssl_context = ssl.ssl_context().to_owned();
        let Some(callback) = ssl_context.ex_data(SslContext::cached_ex_index::<F>()) else {
            return;
        };

        callback(ssl, &line);
    })
}

pub(super) unsafe extern "C" fn raw_sign<M>(
    ssl: *mut ffi::SSL,
    out: *mut u8,
    out_len: *mut usize,
    max_out: usize,
    signature_algorithm: u16,
    in_: *const u8,
    in_len: usize,
) -> ffi::ssl_private_key_result_t
where
    M: PrivateKeyMethod,
{
    catch_ffi(ffi::ssl_private_key_result_t::ssl_private_key_failure, || {
        // SAFETY: boring passes `in_len` readable bytes at `in_` when `in_len > 0`.
        let input = unsafe { c_slice(in_, in_len) };
        let signature_algorithm = SslSignatureAlgorithm(signature_algorithm);
        let callback = |method: &M, ssl: &mut _, output: &mut _| {
            method.sign(ssl, input, signature_algorithm, output)
        };
        // SAFETY: boring provides valid inputs.
        unsafe { raw_private_key_callback(ssl, out, out_len, max_out, callback) }
    })
}

pub(super) unsafe extern "C" fn raw_decrypt<M>(
    ssl: *mut ffi::SSL,
    out: *mut u8,
    out_len: *mut usize,
    max_out: usize,
    in_: *const u8,
    in_len: usize,
) -> ffi::ssl_private_key_result_t
where
    M: PrivateKeyMethod,
{
    catch_ffi(ffi::ssl_private_key_result_t::ssl_private_key_failure, || {
        // SAFETY: boring passes `in_len` readable bytes at `in_` when `in_len > 0`.
        let input = unsafe { c_slice(in_, in_len) };
        let callback =
            |method: &M, ssl: &mut _, output: &mut _| method.decrypt(ssl, input, output);
        // SAFETY: boring provides valid inputs.
        unsafe { raw_private_key_callback(ssl, out, out_len, max_out, callback) }
    })
}

pub(super) unsafe extern "C" fn raw_complete<M>(
    ssl: *mut ffi::SSL,
    out: *mut u8,
    out_len: *mut usize,
    max_out: usize,
) -> ffi::ssl_private_key_result_t
where
    M: PrivateKeyMethod,
{
    catch_ffi(ffi::ssl_private_key_result_t::ssl_private_key_failure, || {
        // SAFETY: boring provides valid inputs.
        unsafe { raw_private_key_callback::<M>(ssl, out, out_len, max_out, M::complete) }
    })
}

unsafe fn raw_private_key_callback<M>(
    ssl: *mut ffi::SSL,
    out: *mut u8,
    out_len: *mut usize,
    max_out: usize,
    callback: impl FnOnce(&M, &mut SslRef, &mut [u8]) -> Result<usize, PrivateKeyMethodError>,
) -> ffi::ssl_private_key_result_t
where
    M: PrivateKeyMethod,
{
    if ssl.is_null() || out_len.is_null() {
        return ffi::ssl_private_key_result_t::ssl_private_key_failure;
    }
    // SAFETY: boring provides valid non-null inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    // SAFETY: boring provides a writable buffer of `max_out` bytes when `max_out > 0`.
    let output = unsafe { c_slice_mut(out, max_out) };
    // SAFETY: `out_len` is checked non-null above.
    let out_len = unsafe { &mut *out_len };

    let ssl_context = ssl.ssl_context().to_owned();
    let Some(method) = ssl_context.ex_data(SslContext::cached_ex_index::<M>()) else {
        return ffi::ssl_private_key_result_t::ssl_private_key_failure;
    };

    match callback(method, ssl, output) {
        Ok(written) => {
            if written > max_out {
                return ffi::ssl_private_key_result_t::ssl_private_key_failure;
            }
            *out_len = written;
            ffi::ssl_private_key_result_t::ssl_private_key_success
        }
        Err(err) => err.0,
    }
}

pub(super) unsafe extern "C" fn raw_ssl_cert_compress<C>(
    ssl: *mut ffi::SSL,
    out: *mut ffi::CBB,
    input: *const u8,
    input_len: usize,
) -> ::std::os::raw::c_int
where
    C: CertificateCompressor,
{
    const {
        assert!(C::CAN_COMPRESS);
    }

    catch_ffi(0, || {
        if ssl.is_null() || out.is_null() {
            return 0;
        }
        // SAFETY: boring provides valid non-null inputs.
        let ssl = unsafe { SslRef::from_ptr_mut(ssl) };

        let ssl_context = ssl.ssl_context().to_owned();
        let Some(compressor) = ssl_context.ex_data(SslContext::cached_ex_index::<C>()) else {
            return 0;
        };

        // SAFETY: boring passes a valid buffer of `input_len` bytes when `input_len > 0`.
        let input_slice = unsafe { c_slice(input, input_len) };
        let mut writer = CryptoByteBuilder::from_ptr(out);
        if compressor.compress(input_slice, &mut writer).is_err() {
            return 0;
        }

        1
    })
}

pub(super) unsafe extern "C" fn raw_ssl_cert_decompress<C>(
    ssl: *mut ffi::SSL,
    out: *mut *mut ffi::CRYPTO_BUFFER,
    uncompressed_len: usize,
    input: *const u8,
    input_len: usize,
) -> ::std::os::raw::c_int
where
    C: CertificateCompressor,
{
    const {
        assert!(C::CAN_DECOMPRESS);
    }

    catch_ffi(0, || {
        if ssl.is_null() || out.is_null() {
            return 0;
        }
        // SAFETY: boring provides valid non-null inputs.
        let ssl = unsafe { SslRef::from_ptr_mut(ssl) };

        let ssl_context = ssl.ssl_context().to_owned();
        let Some(compressor) = ssl_context.ex_data(SslContext::cached_ex_index::<C>()) else {
            return 0;
        };

        let Ok(mut decompression_buffer) = CryptoBufferBuilder::with_capacity(uncompressed_len)
        else {
            return 0;
        };

        // SAFETY: boring passes a valid buffer of `input_len` bytes when `input_len > 0`.
        let input_slice = unsafe { c_slice(input, input_len) };

        if compressor
            .decompress(input_slice, decompression_buffer.as_writer())
            .is_err()
        {
            return 0;
        }

        let Ok(crypto_buffer) = decompression_buffer.build() else {
            return 0;
        };

        // SAFETY: `out` is checked non-null above and receives ownership of `crypto_buffer`.
        unsafe { *out = crypto_buffer };
        1
    })
}

struct CryptoByteBuilder<'a>(*mut ffi::CBB, std::marker::PhantomData<&'a [u8]>);

impl CryptoByteBuilder<'_> {
    fn from_ptr(ptr: *mut ffi::CBB) -> Self {
        Self(ptr, Default::default())
    }
}

impl std::io::Write for CryptoByteBuilder<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // SAFETY: `self.0` is a live `CBB` for this callback and `buf` is valid for `buf.len()`.
        let success = unsafe { ffi::CBB_add_bytes(self.0, buf.as_ptr(), buf.len()) == 1 };
        if !success {
            return Err(std::io::Error::other("CBB_add_bytes failed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // SAFETY: `self.0` is a live `CBB` for this callback.
        let success = unsafe { ffi::CBB_flush(self.0) == 1 };
        if !success {
            return Err(std::io::Error::other("CBB_flush failed"));
        }
        Ok(())
    }
}

struct UninitWriter<'a> {
    buf: &'a mut [MaybeUninit<u8>],
    pos: usize,
}

impl std::io::Write for UninitWriter<'_> {
    fn write(&mut self, src: &[u8]) -> std::io::Result<usize> {
        let remaining = &mut self.buf[self.pos..];
        let n = src.len().min(remaining.len());
        // SAFETY: `src[..n]` is initialized and non-overlapping with `remaining[..n]`.
        unsafe {
            ptr::copy_nonoverlapping(src.as_ptr(), remaining.as_mut_ptr().cast::<u8>(), n);
        }
        self.pos += n;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct CryptoBufferBuilder<'a> {
    buffer: *mut ffi::CRYPTO_BUFFER,
    writer: UninitWriter<'a>,
}

impl<'a> CryptoBufferBuilder<'a> {
    fn with_capacity(capacity: usize) -> Result<CryptoBufferBuilder<'a>, ErrorStack> {
        let mut data: *mut u8 = std::ptr::null_mut();
        // SAFETY: `CRYPTO_BUFFER_alloc` allocates an uninitialized buffer of `capacity` bytes and writes its pointer to `data`.
        let buffer = unsafe { crate::cvt_p(ffi::CRYPTO_BUFFER_alloc(&mut data, capacity))? };
        let buf = if capacity == 0 {
            &mut []
        } else {
            if data.is_null() {
                // SAFETY: `buffer` was just allocated by `CRYPTO_BUFFER_alloc`.
                unsafe { ffi::CRYPTO_BUFFER_free(buffer) };
                return Err(ErrorStack::internal_error_str("null CRYPTO_BUFFER data"));
            }
            // SAFETY: `data` points to `capacity` writable (possibly uninitialized) bytes owned by `buffer`.
            unsafe { slice::from_raw_parts_mut(data.cast::<MaybeUninit<u8>>(), capacity) }
        };
        Ok(CryptoBufferBuilder {
            buffer,
            writer: UninitWriter { buf, pos: 0 },
        })
    }

    fn as_writer(&mut self) -> &mut (impl std::io::Write + 'a) {
        &mut self.writer
    }

    fn build(mut self) -> Result<*mut ffi::CRYPTO_BUFFER, ErrorStack> {
        // SAFETY: `self.buffer` is live until `mem::replace` transfers ownership.
        let buffer_capacity = unsafe { ffi::CRYPTO_BUFFER_len(self.buffer) };
        if self.writer.pos != buffer_capacity {
            return Err(ErrorStack::internal_error_str("invalid len"));
        }
        Ok(mem::replace(&mut self.buffer, ptr::null_mut()))
    }
}

impl Drop for CryptoBufferBuilder<'_> {
    fn drop(&mut self) {
        if !self.buffer.is_null() {
            // SAFETY: `self.buffer` is owned by `self` and has not been transferred out by `build`.
            unsafe {
                leyline_bssl_sys::CRYPTO_BUFFER_free(self.buffer);
            }
        }
    }
}
