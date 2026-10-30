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
use std::mem::{self};
use std::ptr;
use std::slice;
use std::str;
use std::sync::Arc;

pub extern "C" fn raw_verify<F>(preverify_ok: c_int, x509_ctx: *mut ffi::X509_STORE_CTX) -> c_int
where
    F: Fn(bool, &mut X509StoreContextRef) -> bool + 'static + Sync + Send,
{
    // SAFETY: boring provides valid inputs.
    let ctx = unsafe { X509StoreContextRef::from_ptr_mut(x509_ctx) };

    let ssl_idx = X509StoreContext::ssl_idx().expect("BUG: store context ssl index missing");
    let verify_idx = SslContext::cached_ex_index::<F>();

    let verify = ctx
        .ex_data(ssl_idx)
        .expect("BUG: store context missing ssl")
        .ssl_context()
        .ex_data(verify_idx)
        .expect("BUG: verify callback missing");

    // SAFETY: The callback won't outlive the context it's associated with
    let verify = unsafe { &*std::ptr::from_ref::<F>(verify) };

    c_int::from(verify(preverify_ok != 0, ctx))
}

pub(super) unsafe extern "C" fn raw_custom_verify<F>(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
) -> ffi::ssl_verify_result_t
where
    F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
{
    let callback = |ssl: &mut SslRef| {
        let custom_verify_idx = SslContext::cached_ex_index::<F>();

        let ssl_context = ssl.ssl_context().to_owned();
        let callback = ssl_context
            .ex_data(custom_verify_idx)
            .expect("BUG: custom verify callback missing");

        callback(ssl)
    };

    unsafe { raw_custom_verify_callback(ssl, out_alert, callback) }
}

pub(super) unsafe extern "C" fn ssl_raw_custom_verify<F>(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
) -> ffi::ssl_verify_result_t
where
    F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
{
    let callback = |ssl: &mut SslRef| {
        let callback = ssl
            .ex_data(Ssl::cached_ex_index::<Arc<F>>())
            .expect("BUG: ssl verify callback missing")
            .clone();

        callback(ssl)
    };

    unsafe { raw_custom_verify_callback(ssl, out_alert, callback) }
}

unsafe fn raw_custom_verify_callback(
    ssl: *mut ffi::SSL,
    out_alert: *mut u8,
    callback: impl FnOnce(&mut SslRef) -> Result<(), SslVerifyError>,
) -> ffi::ssl_verify_result_t {
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
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
    // SAFETY: boring provides valid inputs.
    let ctx = unsafe { X509StoreContextRef::from_ptr_mut(x509_ctx) };

    let ssl_idx = X509StoreContext::ssl_idx().expect("BUG: store context ssl index missing");

    let callback = ctx
        .ex_data(ssl_idx)
        .expect("BUG: store context missing ssl")
        .ex_data(Ssl::cached_ex_index::<Arc<F>>())
        .expect("BUG: ssl verify callback missing")
        .clone();

    c_int::from(callback(preverify_ok != 0, ctx))
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
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    let protos = unsafe { slice::from_raw_parts(inbuf, inlen as usize) };
    let out = unsafe { &mut *out };
    let outlen = unsafe { &mut *outlen };

    let ssl_context = ssl.ssl_context().to_owned();
    let callback = ssl_context
        .ex_data(SslContext::cached_ex_index::<F>())
        .expect("BUG: alpn callback missing");

    match callback(ssl, protos) {
        Ok(proto) => {
            *out = proto.as_ptr();
            *outlen = proto.len() as c_uchar;

            ffi::SSL_TLSEXT_ERR_OK
        }
        Err(e) => e.0,
    }
}

pub(super) unsafe extern "C" fn raw_select_cert<F>(
    client_hello: *const ffi::SSL_CLIENT_HELLO,
) -> ffi::ssl_select_cert_result_t
where
    F: Fn(ClientHello<'_>) -> Result<(), SelectCertError> + Sync + Send + 'static,
{
    // SAFETY: boring provides valid inputs.
    let client_hello = ClientHello(unsafe { &*client_hello });

    let ssl_context = client_hello.ssl().ssl_context().to_owned();
    let callback = ssl_context
        .ex_data(SslContext::cached_ex_index::<F>())
        .expect("BUG: select cert callback missing");

    match callback(client_hello) {
        Ok(()) => ffi::ssl_select_cert_result_t::ssl_select_cert_success,
        Err(e) => e.0,
    }
}

pub(super) unsafe extern "C" fn raw_new_session<F>(
    ssl: *mut ffi::SSL,
    session: *mut ffi::SSL_SESSION,
) -> c_int
where
    F: Fn(&mut SslRef, SslSession) + 'static + Sync + Send,
{
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    let session = unsafe { SslSession::from_ptr(session) };

    let callback = ssl
        .ex_data(*SESSION_CTX_INDEX)
        .expect("BUG: session context missing")
        .ex_data(SslContext::cached_ex_index::<F>())
        .expect("BUG: new session callback missing");

    // SAFETY: We can make `callback` outlive `ssl` because it is a callback
    let callback = unsafe { &*std::ptr::from_ref::<F>(callback) };

    callback(ssl, session);

    1
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
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    let data = unsafe { slice::from_raw_parts(data, len as usize) };
    let copy = unsafe { &mut *copy };

    let callback = ssl
        .ex_data(*SESSION_CTX_INDEX)
        .expect("BUG: session context missing")
        .ex_data(SslContext::cached_ex_index::<F>())
        .expect("BUG: get session callback missing");

    // SAFETY: We can make `callback` outlive `ssl` because it is a callback
    let callback = unsafe { &*std::ptr::from_ref::<F>(callback) };

    match callback(ssl, data) {
        Ok(Some(session)) => {
            let p = session.into_ptr();

            *copy = 0;

            p
        }
        Ok(None) => ptr::null_mut(),
        Err(GetSessionPendingError) => unsafe { ffi::SSL_magic_pending_session_ptr() },
    }
}

pub(super) unsafe extern "C" fn raw_keylog<F>(ssl: *const ffi::SSL, line: *const c_char)
where
    F: Fn(&SslRef, &str) + 'static + Sync + Send,
{
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr(ssl.cast_mut()) };
    let line = unsafe { CStr::from_ptr(line).to_string_lossy() };

    let callback = ssl
        .ssl_context()
        .ex_data(SslContext::cached_ex_index::<F>())
        .expect("BUG: get session callback missing");

    callback(ssl, &line);
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
    // SAFETY: boring provides valid inputs.
    let input = unsafe { slice::from_raw_parts(in_, in_len) };

    let signature_algorithm = SslSignatureAlgorithm(signature_algorithm);

    let callback = |method: &M, ssl: &mut _, output: &mut _| {
        method.sign(ssl, input, signature_algorithm, output)
    };

    // SAFETY: boring provides valid inputs.
    unsafe { raw_private_key_callback(ssl, out, out_len, max_out, callback) }
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
    // SAFETY: boring provides valid inputs.
    let input = unsafe { slice::from_raw_parts(in_, in_len) };

    let callback = |method: &M, ssl: &mut _, output: &mut _| method.decrypt(ssl, input, output);

    // SAFETY: boring provides valid inputs.
    unsafe { raw_private_key_callback(ssl, out, out_len, max_out, callback) }
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
    // SAFETY: boring provides valid inputs.
    unsafe { raw_private_key_callback::<M>(ssl, out, out_len, max_out, M::complete) }
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
    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };
    let output = unsafe { slice::from_raw_parts_mut(out, max_out) };
    let out_len = unsafe { &mut *out_len };

    let ssl_context = ssl.ssl_context().to_owned();
    let method = ssl_context
        .ex_data(SslContext::cached_ex_index::<M>())
        .expect("BUG: private key method missing");

    match callback(method, ssl, output) {
        Ok(written) => {
            assert!(written <= max_out);

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

    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };

    let ssl_context = ssl.ssl_context();
    let compressor = ssl_context
        .ex_data(SslContext::cached_ex_index::<C>())
        .expect("BUG: certificate compression missed");

    let input_slice = unsafe { std::slice::from_raw_parts(input, input_len) };
    let mut writer = CryptoByteBuilder::from_ptr(out);
    if compressor.compress(input_slice, &mut writer).is_err() {
        return 0;
    }

    1
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

    // SAFETY: boring provides valid inputs.
    let ssl = unsafe { SslRef::from_ptr_mut(ssl) };

    let ssl_context = ssl.ssl_context();
    let compressor = ssl_context
        .ex_data(SslContext::cached_ex_index::<C>())
        .expect("BUG: certificate compression missed");

    let Ok(mut decompression_buffer) = CryptoBufferBuilder::with_capacity(uncompressed_len) else {
        return 0;
    };

    let input_slice = unsafe { std::slice::from_raw_parts(input, input_len) };

    if compressor
        .decompress(input_slice, decompression_buffer.as_writer())
        .is_err()
    {
        return 0;
    }

    let Ok(crypto_buffer) = decompression_buffer.build() else {
        return 0;
    };

    unsafe { *out = crypto_buffer };
    1
}

struct CryptoByteBuilder<'a>(*mut ffi::CBB, std::marker::PhantomData<&'a [u8]>);

impl CryptoByteBuilder<'_> {
    fn from_ptr(ptr: *mut ffi::CBB) -> Self {
        Self(ptr, Default::default())
    }
}

impl std::io::Write for CryptoByteBuilder<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let success = unsafe { ffi::CBB_add_bytes(self.0, buf.as_ptr(), buf.len()) == 1 };
        if !success {
            return Err(std::io::Error::other("CBB_add_bytes failed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let success = unsafe { ffi::CBB_flush(self.0) == 1 };
        if !success {
            return Err(std::io::Error::other("CBB_flush failed"));
        }
        Ok(())
    }
}

struct CryptoBufferBuilder<'a> {
    buffer: *mut ffi::CRYPTO_BUFFER,
    cursor: std::io::Cursor<&'a mut [u8]>,
}

impl<'a> CryptoBufferBuilder<'a> {
    fn with_capacity(capacity: usize) -> Result<CryptoBufferBuilder<'a>, ErrorStack> {
        let mut data: *mut u8 = std::ptr::null_mut();
        let buffer = unsafe { crate::cvt_p(ffi::CRYPTO_BUFFER_alloc(&mut data, capacity))? };
        Ok(CryptoBufferBuilder {
            buffer,
            cursor: std::io::Cursor::new(unsafe { std::slice::from_raw_parts_mut(data, capacity) }),
        })
    }

    fn as_writer(&mut self) -> &mut (impl std::io::Write + 'a) {
        &mut self.cursor
    }

    fn build(mut self) -> Result<*mut ffi::CRYPTO_BUFFER, ErrorStack> {
        let buffer_capacity = unsafe { ffi::CRYPTO_BUFFER_len(self.buffer) };
        if self.cursor.position() != buffer_capacity as u64 {
            return Err(ErrorStack::internal_error_str("invalid len"));
        }
        Ok(mem::replace(&mut self.buffer, ptr::null_mut()))
    }
}

impl Drop for CryptoBufferBuilder<'_> {
    fn drop(&mut self) {
        if !self.buffer.is_null() {
            unsafe {
                leyline_bssl_sys::CRYPTO_BUFFER_free(self.buffer);
            }
        }
    }
}
