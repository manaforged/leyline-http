use foreign_types::{ForeignType, ForeignTypeRef, Opaque};
use openssl_macros::corresponds;
use std::any::TypeId;
use std::collections::HashMap;
use std::ffi::{c_int, c_uchar, c_uint};
use std::ffi::{CStr, CString};
use std::fmt;
use std::io;
use std::io::prelude::*;
use std::marker::PhantomData;
use std::mem::{self, ManuallyDrop, MaybeUninit};
use std::ops::Deref;
use std::panic::resume_unwind;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::slice;
use std::str;
use std::sync::{Arc, LazyLock, Mutex};

use crate::dh::DhRef;
use crate::error::ErrorStack;
use crate::ex_data::Index;
use crate::pkey::{HasPrivate, PKeyRef, Params};
use crate::ssl::bio::BioMethod;
use crate::ssl::callbacks::*;
use crate::ssl::error::InnerError;
use crate::stack::{StackRef, Stackable};
use crate::try_int;
use crate::x509::store::{X509Store, X509StoreBuilderRef, X509StoreRef};
use crate::x509::verify::X509VerifyParamRef;
use crate::x509::{X509Ref, X509StoreContextRef, X509VerifyError, X509VerifyResult, X509};
use crate::{cvt, cvt_0i, cvt_n, cvt_p, init};
use crate::{ffi, free_data_box};

pub use self::async_callbacks::{
    AsyncPrivateKeyMethod, AsyncPrivateKeyMethodError, AsyncSelectCertError, BoxCustomVerifyFinish,
    BoxCustomVerifyFuture, BoxGetSessionFinish, BoxGetSessionFuture, BoxPrivateKeyMethodFinish,
    BoxPrivateKeyMethodFuture, BoxSelectCertFinish, BoxSelectCertFuture, ExDataFuture,
};
pub use self::connector::{
    ConnectConfiguration, SslAcceptor, SslAcceptorBuilder, SslConnector, SslConnectorBuilder,
};
pub use self::ech::{SslEchKeys, SslEchKeysRef};
pub use self::error::{Error, ErrorCode, HandshakeError};

mod async_callbacks;
mod bio;
mod callbacks;
mod chain_pem;
mod connector;
mod ech;
mod error;
mod key_shares;
mod mut_only;

bitflags! {
    #[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
    pub struct SslOptions: c_uint {
        const DONT_INSERT_EMPTY_FRAGMENTS = ffi::SSL_OP_DONT_INSERT_EMPTY_FRAGMENTS as _;

        const ALL = ffi::SSL_OP_ALL as _;

        const NO_QUERY_MTU = ffi::SSL_OP_NO_QUERY_MTU as _;

        const NO_TICKET = ffi::SSL_OP_NO_TICKET as _;

        const NO_SESSION_RESUMPTION_ON_RENEGOTIATION =
            ffi::SSL_OP_NO_SESSION_RESUMPTION_ON_RENEGOTIATION as _;

        const NO_COMPRESSION = ffi::SSL_OP_NO_COMPRESSION as _;

        const ALLOW_UNSAFE_LEGACY_RENEGOTIATION =
            ffi::SSL_OP_ALLOW_UNSAFE_LEGACY_RENEGOTIATION as _;

        const SINGLE_ECDH_USE = ffi::SSL_OP_SINGLE_ECDH_USE as _;

        const SINGLE_DH_USE = ffi::SSL_OP_SINGLE_DH_USE as _;

        const CIPHER_SERVER_PREFERENCE = ffi::SSL_OP_CIPHER_SERVER_PREFERENCE as _;

        const TLS_ROLLBACK_BUG = ffi::SSL_OP_TLS_ROLLBACK_BUG as _;

        const NO_SSLV2 = ffi::SSL_OP_NO_SSLv2 as _;

        const NO_SSLV3 = ffi::SSL_OP_NO_SSLv3 as _;

        const NO_TLSV1 = ffi::SSL_OP_NO_TLSv1 as _;

        const NO_TLSV1_1 = ffi::SSL_OP_NO_TLSv1_1 as _;

        const NO_TLSV1_2 = ffi::SSL_OP_NO_TLSv1_2 as _;

        const NO_TLSV1_3 = ffi::SSL_OP_NO_TLSv1_3 as _;

        const NO_DTLSV1 = ffi::SSL_OP_NO_DTLSv1 as _;

        const NO_DTLSV1_2 = ffi::SSL_OP_NO_DTLSv1_2 as _;

        const NO_RENEGOTIATION = ffi::SSL_OP_NO_RENEGOTIATION as _;
    }
}

bitflags! {
    #[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
    pub struct SslMode: c_uint {
        const ENABLE_PARTIAL_WRITE = ffi::SSL_MODE_ENABLE_PARTIAL_WRITE as _;

        const ACCEPT_MOVING_WRITE_BUFFER = ffi::SSL_MODE_ACCEPT_MOVING_WRITE_BUFFER as _;

        const AUTO_RETRY = ffi::SSL_MODE_AUTO_RETRY as _;

        const NO_AUTO_CHAIN = ffi::SSL_MODE_NO_AUTO_CHAIN as _;

        const RELEASE_BUFFERS = ffi::SSL_MODE_RELEASE_BUFFERS as _;

        const SEND_FALLBACK_SCSV = ffi::SSL_MODE_SEND_FALLBACK_SCSV as _;
    }
}

#[derive(Copy, Clone)]
pub struct SslMethod {
    ptr: *const ffi::SSL_METHOD,
    is_x509_method: bool,
}

impl SslMethod {
    #[corresponds(TLS_method)]
    #[must_use]
    pub fn tls() -> Self {
        unsafe {
            Self {
                ptr: ffi::TLS_method(),
                is_x509_method: true,
            }
        }
    }

    #[corresponds(TLS_server_method)]
    #[must_use]
    pub unsafe fn from_ptr(ptr: *const ffi::SSL_METHOD) -> SslMethod {
        SslMethod {
            ptr,
            is_x509_method: false,
        }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_ptr(&self) -> *const ffi::SSL_METHOD {
        self.ptr
    }
}

unsafe impl Sync for SslMethod {}
unsafe impl Send for SslMethod {}

bitflags! {
    #[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
    pub struct SslVerifyMode: i32 {
        const PEER = ffi::SSL_VERIFY_PEER;

        const NONE = ffi::SSL_VERIFY_NONE;

        const FAIL_IF_NO_PEER_CERT = ffi::SSL_VERIFY_FAIL_IF_NO_PEER_CERT;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SslVerifyError {
    Invalid(SslAlert),
    Retry,
}

bitflags! {
    #[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
    pub struct SslSessionCacheMode: c_int {
        const OFF = ffi::SSL_SESS_CACHE_OFF;

        const CLIENT = ffi::SSL_SESS_CACHE_CLIENT;

        const SERVER = ffi::SSL_SESS_CACHE_SERVER;

        const BOTH = ffi::SSL_SESS_CACHE_BOTH;

        const NO_AUTO_CLEAR = ffi::SSL_SESS_CACHE_NO_AUTO_CLEAR;

        const NO_INTERNAL_LOOKUP = ffi::SSL_SESS_CACHE_NO_INTERNAL_LOOKUP;

        const NO_INTERNAL_STORE = ffi::SSL_SESS_CACHE_NO_INTERNAL_STORE;

        const NO_INTERNAL = ffi::SSL_SESS_CACHE_NO_INTERNAL;
    }
}

#[derive(Copy, Clone)]
pub struct SslFiletype(c_int);

impl SslFiletype {
    pub const PEM: SslFiletype = SslFiletype(ffi::SSL_FILETYPE_PEM);

    #[must_use]
    pub fn from_raw(raw: c_int) -> SslFiletype {
        SslFiletype(raw)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

#[derive(Copy, Clone)]
pub struct StatusType(c_int);

impl StatusType {
    #[must_use]
    pub fn from_raw(raw: c_int) -> StatusType {
        StatusType(raw)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

#[derive(Copy, Clone)]
pub struct NameType(c_int);

impl NameType {
    pub const HOST_NAME: NameType = NameType(ffi::TLSEXT_NAMETYPE_host_name);

    #[must_use]
    pub fn from_raw(raw: c_int) -> StatusType {
        StatusType(raw)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_raw(&self) -> c_int {
        self.0
    }
}

static INDEXES: LazyLock<Mutex<HashMap<TypeId, c_int>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SSL_INDEXES: LazyLock<Mutex<HashMap<TypeId, c_int>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SESSION_CTX_INDEX: LazyLock<Index<Ssl, SslContext>> =
    LazyLock::new(|| Ssl::new_ex_index().unwrap());
static X509_FLAG_INDEX: LazyLock<Index<SslContext, bool>> =
    LazyLock::new(|| SslContext::new_ex_index().unwrap());

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SniError(c_int);

impl SniError {
    pub const NOACK: SniError = SniError(ffi::SSL_TLSEXT_ERR_NOACK);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SslAlert(c_int);

impl SslAlert {
    pub const CERTIFICATE_UNKNOWN: Self = Self(ffi::SSL_AD_CERTIFICATE_UNKNOWN);
    pub const PROTOCOL_VERSION: Self = Self(ffi::SSL_AD_PROTOCOL_VERSION);
    pub const INTERNAL_ERROR: Self = Self(ffi::SSL_AD_INTERNAL_ERROR);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AlpnError(c_int);

impl AlpnError {
    pub const NOACK: AlpnError = AlpnError(ffi::SSL_TLSEXT_ERR_NOACK);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SelectCertError(ffi::ssl_select_cert_result_t);

impl SelectCertError {
    pub const ERROR: Self = Self(ffi::ssl_select_cert_result_t::ssl_select_cert_error);

    pub const RETRY: Self = Self(ffi::ssl_select_cert_result_t::ssl_select_cert_retry);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ExtensionType(u16);

impl ExtensionType {
    pub const SERVER_NAME: Self = Self(ffi::TLSEXT_TYPE_server_name as u16);
    pub const PADDING: Self = Self(ffi::TLSEXT_TYPE_padding as u16);
    pub const PRE_SHARED_KEY: Self = Self(ffi::TLSEXT_TYPE_pre_shared_key as u16);
    pub const COOKIE: Self = Self(ffi::TLSEXT_TYPE_cookie as u16);
}

impl From<u16> for ExtensionType {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct SslVersion(u16);

impl SslVersion {
    pub const SSL3: SslVersion = SslVersion(ffi::SSL3_VERSION as _);

    pub const TLS1: SslVersion = SslVersion(ffi::TLS1_VERSION as _);

    pub const TLS1_1: SslVersion = SslVersion(ffi::TLS1_1_VERSION as _);

    pub const TLS1_2: SslVersion = SslVersion(ffi::TLS1_2_VERSION as _);

    pub const TLS1_3: SslVersion = SslVersion(ffi::TLS1_3_VERSION as _);

    pub const DTLS1: SslVersion = SslVersion(ffi::DTLS1_VERSION as _);

    pub const DTLS1_2: SslVersion = SslVersion(ffi::DTLS1_2_VERSION as _);

    pub const DTLS1_3: SslVersion = SslVersion(ffi::DTLS1_3_VERSION as _);
}

impl TryFrom<u16> for SslVersion {
    type Error = &'static str;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match i32::from(value) {
            ffi::SSL3_VERSION
            | ffi::TLS1_VERSION
            | ffi::TLS1_1_VERSION
            | ffi::TLS1_2_VERSION
            | ffi::TLS1_3_VERSION
            | ffi::DTLS1_VERSION
            | ffi::DTLS1_2_VERSION
            | ffi::DTLS1_3_VERSION => Ok(Self(value)),
            _ => Err("Unknown SslVersion"),
        }
    }
}

impl fmt::Debug for SslVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            Self::SSL3 => "SSL3",
            Self::TLS1 => "TLS1",
            Self::TLS1_1 => "TLS1_1",
            Self::TLS1_2 => "TLS1_2",
            Self::TLS1_3 => "TLS1_3",
            Self::DTLS1 => "DTLS1",
            Self::DTLS1_2 => "DTLS1_2",
            Self::DTLS1_3 => "DTLS1_3",
            _ => return write!(f, "{:#06x}", self.0),
        })
    }
}

impl fmt::Display for SslVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            Self::SSL3 => "SSLv3",
            Self::TLS1 => "TLSv1",
            Self::TLS1_1 => "TLSv1.1",
            Self::TLS1_2 => "TLSv1.2",
            Self::TLS1_3 => "TLSv1.3",
            Self::DTLS1 => "DTLSv1.0",
            Self::DTLS1_2 => "DTLSv1.2",
            Self::DTLS1_3 => "DTLSv1.3",
            _ => return write!(f, "unknown ({:#06x})", self.0),
        })
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SslSignatureAlgorithm(u16);

impl SslSignatureAlgorithm {
    #[corresponds(SSL_get_signature_algorithm_name)]
    #[must_use]
    pub fn name(&self) -> Option<&'static str> {
        unsafe {
            let ptr = ffi::SSL_get_signature_algorithm_name(self.0, 1);
            if ptr.is_null() {
                None
            } else {
                CStr::from_ptr(ptr).to_str().ok()
            }
        }
    }
}

impl fmt::Display for SslSignatureAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "unknown ({:#06x})", self.0),
        }
    }
}

impl From<u16> for SslSignatureAlgorithm {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CompliancePolicy(ffi::ssl_compliance_policy_t);

impl CompliancePolicy {
    pub const NONE: Self = Self(ffi::ssl_compliance_policy_t::ssl_compliance_policy_none);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CertificateCompressionAlgorithm(u16);

impl CertificateCompressionAlgorithm {
    pub const ZLIB: Self = Self(ffi::TLSEXT_cert_compression_zlib as u16);

    pub const BROTLI: Self = Self(ffi::TLSEXT_cert_compression_brotli as u16);

    pub const ZSTD: Self = Self(ffi::TLSEXT_cert_compression_zstd as u16);
}

#[corresponds(SSL_select_next_proto)]
#[must_use]
pub fn select_next_proto<'a>(server: &'a [u8], client: &'a [u8]) -> Option<&'a [u8]> {
    if server.is_empty() || client.is_empty() {
        return None;
    }

    unsafe {
        let mut out = ptr::null_mut();
        let mut outlen = 0;
        let r = ffi::SSL_select_next_proto(
            &mut out,
            &mut outlen,
            server.as_ptr(),
            try_int(server.len()).ok()?,
            client.as_ptr(),
            try_int(client.len()).ok()?,
        );

        if r == ffi::OPENSSL_NPN_NEGOTIATED {
            Some(slice::from_raw_parts(out.cast_const(), outlen as usize))
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
pub struct SslInfoCallbackMode(i32);

impl SslInfoCallbackMode {
    pub const HANDSHAKE_DONE: Self = Self(ffi::SSL_CB_HANDSHAKE_DONE);
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
pub enum SslInfoCallbackValue {
    Unit,
    Alert(SslInfoCallbackAlert),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TicketKeyCallbackResult {
    Error,

    Noop,

    Success,

    DecryptSuccessRenew,
}

impl From<TicketKeyCallbackResult> for c_int {
    fn from(value: TicketKeyCallbackResult) -> Self {
        match value {
            TicketKeyCallbackResult::Error => -1,
            TicketKeyCallbackResult::Noop => 0,
            TicketKeyCallbackResult::Success => 1,
            TicketKeyCallbackResult::DecryptSuccessRenew => 2,
        }
    }
}

#[derive(Hash, Copy, Clone, PartialOrd, Ord, Eq, PartialEq, Debug)]
pub struct SslInfoCallbackAlert(c_int);

impl SslInfoCallbackAlert {
    #[must_use]
    pub fn alert(&self) -> SslAlert {
        let value = self.0 & i32::from(u8::MAX);
        SslAlert(value)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Ssl3AlertLevel(c_int);

pub struct SslContextBuilder {
    ctx: SslContext,
    has_shared_cert_store: bool,
}

impl SslContextBuilder {
    #[corresponds(SSL_CTX_new)]
    pub fn new(method: SslMethod) -> Result<SslContextBuilder, ErrorStack> {
        unsafe {
            init();
            let ctx = cvt_p(ffi::SSL_CTX_new(method.as_ptr()))?;
            let mut builder = SslContextBuilder::from_ptr(ctx);

            if method.is_x509_method {
                builder.ctx.assume_x509();
            }

            Ok(builder)
        }
    }

    pub unsafe fn from_ptr(ctx: *mut ffi::SSL_CTX) -> Self {
        Self {
            ctx: unsafe { SslContext::from_ptr(ctx) },
            has_shared_cert_store: false,
        }
    }

    #[must_use]
    pub fn as_ptr(&self) -> *mut ffi::SSL_CTX {
        self.ctx.as_ptr()
    }

    #[corresponds(SSL_CTX_set_verify)]
    pub fn set_verify(&mut self, mode: SslVerifyMode) {
        self.ctx.check_x509();

        unsafe {
            ffi::SSL_CTX_set_verify(self.as_ptr(), c_int::from(mode.bits()), None);
        }
    }

    #[corresponds(SSL_CTX_set_verify)]
    pub fn set_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(bool, &mut X509StoreContextRef) -> bool + 'static + Sync + Send,
    {
        self.ctx.check_x509();

        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_set_verify(
                self.as_ptr(),
                c_int::from(mode.bits()),
                Some(raw_verify::<F>),
            );
        }
    }

    #[corresponds(SSL_CTX_set_custom_verify)]
    pub fn set_custom_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_set_custom_verify(
                self.as_ptr(),
                c_int::from(mode.bits()),
                Some(raw_custom_verify::<F>),
            );
        }
    }

    #[corresponds(SSL_CTX_set_cert_store)]
    pub fn set_cert_store(&mut self, cert_store: X509Store) {
        self.ctx.check_x509();

        self.has_shared_cert_store = true;
        unsafe {
            ffi::SSL_CTX_set_cert_store(self.as_ptr(), cert_store.into_ptr());
        }
    }

    #[corresponds(SSL_CTX_set_cert_store)]
    pub fn set_cert_store_ref(&mut self, cert_store: &X509Store) {
        self.set_cert_store(cert_store.to_owned());
    }

    #[corresponds(SSL_CTX_set_mode)]
    pub fn set_mode(&mut self, mode: SslMode) -> SslMode {
        let bits = unsafe { ffi::SSL_CTX_set_mode(self.as_ptr(), mode.bits()) };
        SslMode::from_bits_retain(bits)
    }

    #[corresponds(SSL_CTX_set_tmp_dh)]
    pub fn set_tmp_dh(&mut self, dh: &DhRef<Params>) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::SSL_CTX_set_tmp_dh(self.as_ptr(), dh.as_ptr())) }
    }

    #[corresponds(SSL_CTX_set_default_verify_paths)]
    pub fn set_default_verify_paths(&mut self) -> Result<(), ErrorStack> {
        self.ctx.check_x509();

        unsafe { cvt(ffi::SSL_CTX_set_default_verify_paths(self.as_ptr())) }
    }

    #[corresponds(SSL_CTX_load_verify_locations)]
    pub fn set_ca_file<P: AsRef<Path>>(&mut self, file: P) -> Result<(), ErrorStack> {
        self.load_verify_locations(Some(file.as_ref()), None)
    }

    #[corresponds(SSL_CTX_load_verify_locations)]
    pub fn load_verify_locations(
        &mut self,
        ca_file: Option<&Path>,
        ca_path: Option<&Path>,
    ) -> Result<(), ErrorStack> {
        self.ctx.check_x509();

        let ca_file = ca_file.map(path_to_cstring).transpose()?;
        let ca_path = ca_path.map(path_to_cstring).transpose()?;

        unsafe {
            cvt(ffi::SSL_CTX_load_verify_locations(
                self.as_ptr(),
                ca_file.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                ca_path.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            ))
        }
    }

    #[corresponds(SSL_CTX_use_certificate_chain_file)]
    pub fn set_certificate_chain_file<P: AsRef<Path>>(
        &mut self,
        file: P,
    ) -> Result<(), ErrorStack> {
        let file = path_to_cstring(file.as_ref())?;
        unsafe {
            cvt(ffi::SSL_CTX_use_certificate_chain_file(
                self.as_ptr(),
                file.as_ptr(),
            ))
        }
    }

    #[corresponds(SSL_CTX_use_certificate)]
    pub fn set_certificate(&mut self, cert: &X509Ref) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::SSL_CTX_use_certificate(self.as_ptr(), cert.as_ptr())) }
    }

    #[corresponds(SSL_CTX_add_extra_chain_cert)]
    pub fn add_extra_chain_cert(&mut self, cert: X509) -> Result<(), ErrorStack> {
        self.ctx.check_x509();

        unsafe {
            cvt(ffi::SSL_CTX_add_extra_chain_cert(
                self.as_ptr(),
                cert.into_ptr(),
            ))
        }
    }

    #[corresponds(SSL_CTX_use_PrivateKey_file)]
    pub fn set_private_key_file<P: AsRef<Path>>(
        &mut self,
        file: P,
        file_type: SslFiletype,
    ) -> Result<(), ErrorStack> {
        let file = path_to_cstring(file.as_ref())?;
        unsafe {
            cvt(ffi::SSL_CTX_use_PrivateKey_file(
                self.as_ptr(),
                file.as_ptr(),
                file_type.as_raw(),
            ))
        }
    }

    #[corresponds(SSL_CTX_use_PrivateKey)]
    pub fn set_private_key<T>(&mut self, key: &PKeyRef<T>) -> Result<(), ErrorStack>
    where
        T: HasPrivate,
    {
        unsafe { cvt(ffi::SSL_CTX_use_PrivateKey(self.as_ptr(), key.as_ptr())) }
    }

    #[corresponds(SSL_CTX_set_cipher_list)]
    pub fn set_cipher_list(&mut self, cipher_list: &str) -> Result<(), ErrorStack> {
        let cipher_list = CString::new(cipher_list).map_err(ErrorStack::internal_error)?;
        unsafe {
            cvt(ffi::SSL_CTX_set_cipher_list(
                self.as_ptr(),
                cipher_list.as_ptr(),
            ))
        }
    }

    #[corresponds(SSL_CTX_get_ciphers)]
    #[must_use]
    pub fn ciphers(&self) -> Option<&StackRef<SslCipher>> {
        self.ctx.ciphers()
    }

    #[corresponds(SSL_CTX_set_options)]
    pub fn set_options(&mut self, option: SslOptions) -> SslOptions {
        let bits = unsafe { ffi::SSL_CTX_set_options(self.as_ptr(), option.bits()) };
        SslOptions::from_bits_retain(bits)
    }

    #[corresponds(SSL_CTX_get_options)]
    #[must_use]
    pub fn options(&self) -> SslOptions {
        let bits = unsafe { ffi::SSL_CTX_get_options(self.as_ptr()) };
        SslOptions::from_bits_retain(bits)
    }

    #[corresponds(SSL_CTX_set_min_proto_version)]
    pub fn set_min_proto_version(&mut self, version: Option<SslVersion>) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_min_proto_version(
                self.as_ptr(),
                version.map_or(0, |v| v.0 as _),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_max_proto_version)]
    pub fn set_max_proto_version(&mut self, version: Option<SslVersion>) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_max_proto_version(
                self.as_ptr(),
                version.map_or(0, |v| v.0 as _),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_alpn_protos)]
    pub fn set_alpn_protos(&mut self, protocols: &[u8]) -> Result<(), ErrorStack> {
        unsafe {
            let r = ffi::SSL_CTX_set_alpn_protos(
                self.as_ptr(),
                protocols.as_ptr(),
                try_int(protocols.len())?,
            );
            if r == 0 {
                Ok(())
            } else {
                Err(ErrorStack::get())
            }
        }
    }

    #[corresponds(SSL_CTX_set_alpn_select_cb)]
    pub fn set_alpn_select_callback<F>(&mut self, callback: F)
    where
        F: for<'a> Fn(&mut SslRef, &'a [u8]) -> Result<&'a [u8], AlpnError> + 'static + Sync + Send,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_set_alpn_select_cb(
                self.as_ptr(),
                Some(callbacks::raw_alpn_select::<F>),
                ptr::null_mut(),
            );
        }
    }

    #[corresponds(SSL_CTX_set_select_certificate_cb)]
    pub fn set_select_certificate_callback<F>(&mut self, callback: F)
    where
        F: Fn(ClientHello<'_>) -> Result<(), SelectCertError> + Sync + Send + 'static,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_set_select_certificate_cb(
                self.as_ptr(),
                Some(callbacks::raw_select_cert::<F>),
            );
        }
    }

    #[corresponds(SSL_CTX_add_cert_compression_alg)]
    pub fn add_certificate_compression_algorithm<C>(
        &mut self,
        compressor: C,
    ) -> Result<(), ErrorStack>
    where
        C: CertificateCompressor,
    {
        const {
            assert!(C::CAN_COMPRESS || C::CAN_DECOMPRESS, "Either compression or decompression must be supported for algorithm to be registered");
        };
        let success = unsafe {
            ffi::SSL_CTX_add_cert_compression_alg(
                self.as_ptr(),
                C::ALGORITHM.0,
                const {
                    if C::CAN_COMPRESS {
                        Some(callbacks::raw_ssl_cert_compress::<C>)
                    } else {
                        None
                    }
                },
                const {
                    if C::CAN_DECOMPRESS {
                        Some(callbacks::raw_ssl_cert_decompress::<C>)
                    } else {
                        None
                    }
                },
            ) == 1
        };
        if !success {
            return Err(ErrorStack::get());
        }
        self.replace_ex_data(SslContext::cached_ex_index::<C>(), compressor);
        Ok(())
    }

    #[corresponds(SSL_CTX_set_private_key_method)]
    pub fn set_private_key_method<M>(&mut self, method: M)
    where
        M: PrivateKeyMethod,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<M>(), method);

            ffi::SSL_CTX_set_private_key_method(
                self.as_ptr(),
                &ffi::SSL_PRIVATE_KEY_METHOD {
                    sign: Some(callbacks::raw_sign::<M>),
                    decrypt: Some(callbacks::raw_decrypt::<M>),
                    complete: Some(callbacks::raw_complete::<M>),
                },
            );
        }
    }

    #[corresponds(SSL_CTX_get_cert_store)]
    #[must_use]
    pub fn cert_store(&self) -> &X509StoreBuilderRef {
        self.ctx.check_x509();

        unsafe { X509StoreBuilderRef::from_ptr(ffi::SSL_CTX_get_cert_store(self.as_ptr())) }
    }

    #[corresponds(SSL_CTX_get_cert_store)]
    pub fn cert_store_mut(&mut self) -> &mut X509StoreBuilderRef {
        self.ctx.check_x509();

        assert!(
            !self.has_shared_cert_store,
            "Shared X509Store can't be mutated. Use set_cert_store_builder() instead of set_cert_store()
                or completely finish building the cert store setting it."
        );

        unsafe { X509StoreBuilderRef::from_ptr_mut(ffi::SSL_CTX_get_cert_store(self.as_ptr())) }
    }

    #[corresponds(SSL_CTX_sess_set_new_cb)]
    pub fn set_new_session_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut SslRef, SslSession) + 'static + Sync + Send,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_sess_set_new_cb(self.as_ptr(), Some(callbacks::raw_new_session::<F>));
        }
    }

    #[corresponds(SSL_CTX_sess_set_get_cb)]
    pub unsafe fn set_get_session_callback<F>(&mut self, callback: F)
    where
        F: Fn(&mut SslRef, &[u8]) -> Result<Option<SslSession>, GetSessionPendingError>
            + 'static
            + Sync
            + Send,
    {
        self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
        unsafe {
            ffi::SSL_CTX_sess_set_get_cb(self.as_ptr(), Some(callbacks::raw_get_session::<F>));
        }
    }

    #[corresponds(SSL_CTX_set_keylog_callback)]
    pub fn set_keylog_callback<F>(&mut self, callback: F)
    where
        F: Fn(&SslRef, &str) + 'static + Sync + Send,
    {
        unsafe {
            self.replace_ex_data(SslContext::cached_ex_index::<F>(), callback);
            ffi::SSL_CTX_set_keylog_callback(self.as_ptr(), Some(callbacks::raw_keylog::<F>));
        }
    }

    #[corresponds(SSL_CTX_set_session_cache_mode)]
    pub fn set_session_cache_mode(&mut self, mode: SslSessionCacheMode) -> SslSessionCacheMode {
        unsafe {
            let bits = ffi::SSL_CTX_set_session_cache_mode(self.as_ptr(), mode.bits());
            SslSessionCacheMode::from_bits_retain(bits)
        }
    }

    #[corresponds(SSL_CTX_set_ex_data)]
    pub fn set_ex_data<T>(&mut self, index: Index<SslContext, T>, data: T) {
        unsafe {
            self.ctx.replace_ex_data(index, data);
        }
    }

    #[corresponds(SSL_CTX_set_ex_data)]
    pub fn replace_ex_data<T>(&mut self, index: Index<SslContext, T>, data: T) -> Option<T> {
        unsafe { self.ctx.replace_ex_data(index, data) }
    }

    #[corresponds(SSL_CTX_set_grease_enabled)]
    pub fn set_grease_enabled(&mut self, enabled: bool) {
        unsafe { ffi::SSL_CTX_set_grease_enabled(self.as_ptr(), enabled as _) }
    }

    #[corresponds(SSL_CTX_set_grease_sigalgs_enabled)]
    pub fn set_grease_sigalgs_enabled(&mut self, enabled: bool) {
        unsafe { ffi::SSL_CTX_set_grease_sigalgs_enabled(self.as_ptr(), enabled as _) }
    }

    #[corresponds(SSL_CTX_set_signing_algorithm_prefs)]
    pub fn set_sigalgs(&mut self, sigalgs: &[u16]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_signing_algorithm_prefs(
                self.as_ptr(),
                sigalgs.as_ptr(),
                sigalgs.len(),
            ))?;
            cvt(ffi::SSL_CTX_set_verify_algorithm_prefs(
                self.as_ptr(),
                sigalgs.as_ptr(),
                sigalgs.len(),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_record_size_limit)]
    pub fn set_record_size_limit(&mut self, limit: u16) {
        unsafe { ffi::SSL_CTX_set_record_size_limit(self.as_ptr(), limit as _) }
    }

    #[corresponds(SSL_CTX_set_delegated_credentials)]
    pub fn set_delegated_credentials(&mut self, sigalgs: &str) -> Result<(), ErrorStack> {
        let sigalgs = CString::new(sigalgs).map_err(ErrorStack::internal_error)?;
        unsafe {
            cvt(ffi::SSL_CTX_set_delegated_credentials(self.as_ptr(), sigalgs.as_ptr()) as c_int)
        }
    }

    #[corresponds(SSL_CTX_set_extension_order)]
    pub fn set_extension_order(&mut self, order: &[u16]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_extension_order(
                self.as_ptr(),
                order.as_ptr(),
                order.len(),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_extension_tail)]
    pub fn set_extension_tail(&mut self, tail: &[u16]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_extension_tail(
                self.as_ptr(),
                tail.as_ptr(),
                tail.len(),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_tls13_cipher_order)]
    pub fn set_tls13_cipher_order(&mut self, order: &[u16]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_CTX_set_tls13_cipher_order(
                self.as_ptr(),
                order.as_ptr(),
                order.len(),
            ))
        }
    }

    #[corresponds(SSL_CTX_set_permute_extensions)]
    pub fn set_permute_extensions(&mut self, enabled: bool) {
        unsafe { ffi::SSL_CTX_set_permute_extensions(self.as_ptr(), enabled as _) }
    }

    #[corresponds(SSL_CTX_enable_signed_cert_timestamps)]
    pub fn enable_signed_cert_timestamps(&mut self) {
        unsafe { ffi::SSL_CTX_enable_signed_cert_timestamps(self.as_ptr()) }
    }

    #[corresponds(SSL_CTX_enable_ocsp_stapling)]
    pub fn enable_ocsp_stapling(&mut self) {
        unsafe { ffi::SSL_CTX_enable_ocsp_stapling(self.as_ptr()) }
    }

    #[corresponds(SSL_CTX_set1_curves_list)]
    pub fn set_curves_list(&mut self, curves: &str) -> Result<(), ErrorStack> {
        let curves = CString::new(curves).map_err(ErrorStack::internal_error)?;
        unsafe {
            cvt_0i(ffi::SSL_CTX_set1_curves_list(
                self.as_ptr(),
                curves.as_ptr(),
            ))
            .map(|_| ())
        }
    }

    #[must_use]
    pub fn build(self) -> SslContext {
        self.ctx
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::SSL_CTX;
    fn drop = ffi::SSL_CTX_free;

    pub struct SslContext;
}

impl Clone for SslContext {
    fn clone(&self) -> Self {
        (**self).to_owned()
    }
}

impl ToOwned for SslContextRef {
    type Owned = SslContext;

    fn to_owned(&self) -> Self::Owned {
        unsafe {
            SSL_CTX_up_ref(self.as_ptr());
            SslContext::from_ptr(self.as_ptr())
        }
    }
}

impl fmt::Debug for SslContext {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "SslContext")
    }
}

impl SslContext {
    pub fn builder(method: SslMethod) -> Result<SslContextBuilder, ErrorStack> {
        SslContextBuilder::new(method)
    }

    #[corresponds(SSL_CTX_get_ex_new_index)]
    pub fn new_ex_index<T>() -> Result<Index<SslContext, T>, ErrorStack>
    where
        T: 'static + Sync + Send,
    {
        unsafe {
            ffi::init();
            let idx = cvt_n(get_new_idx(Some(free_data_box::<T>)))?;
            Ok(Index::from_raw(idx))
        }
    }

    fn cached_ex_index<T>() -> Index<SslContext, T>
    where
        T: 'static + Sync + Send,
    {
        unsafe {
            let idx = *INDEXES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(TypeId::of::<T>())
                .or_insert_with(|| SslContext::new_ex_index::<T>().unwrap().as_raw());
            Index::from_raw(idx)
        }
    }

    #[corresponds(SSL_CTX_get_ciphers)]
    #[must_use]
    pub fn ciphers(&self) -> Option<&StackRef<SslCipher>> {
        unsafe {
            let ciphers = ffi::SSL_CTX_get_ciphers(self.as_ptr());
            if ciphers.is_null() {
                None
            } else {
                Some(StackRef::from_ptr(ciphers))
            }
        }
    }
}

impl SslContextRef {
    #[corresponds(SSL_CTX_get0_certificate)]
    #[must_use]
    pub fn certificate(&self) -> Option<&X509Ref> {
        self.check_x509();

        unsafe {
            let ptr = ffi::SSL_CTX_get0_certificate(self.as_ptr());
            if ptr.is_null() {
                None
            } else {
                Some(X509Ref::from_ptr(ptr))
            }
        }
    }

    #[corresponds(SSL_CTX_get_cert_store)]
    #[must_use]
    pub fn cert_store(&self) -> &X509StoreRef {
        self.check_x509();

        unsafe { X509StoreRef::from_ptr(ffi::SSL_CTX_get_cert_store(self.as_ptr())) }
    }

    #[corresponds(SSL_CTX_get_ex_data)]
    #[must_use]
    pub fn ex_data<T>(&self, index: Index<SslContext, T>) -> Option<&T> {
        unsafe {
            let data = ffi::SSL_CTX_get_ex_data(self.as_ptr(), index.as_raw());
            if data.is_null() {
                None
            } else {
                Some(&*(data as *const T))
            }
        }
    }

    #[corresponds(SSL_CTX_get_ex_data)]
    unsafe fn ex_data_mut<T>(&mut self, index: Index<SslContext, T>) -> Option<&mut T> {
        unsafe {
            ffi::SSL_CTX_get_ex_data(self.as_ptr(), index.as_raw())
                .cast::<T>()
                .as_mut()
        }
    }

    #[corresponds(SSL_CTX_set_ex_data)]
    unsafe fn set_ex_data<T>(&mut self, index: Index<SslContext, T>, data: T) {
        unsafe {
            let data = Box::into_raw(Box::new(data));
            ffi::SSL_CTX_set_ex_data(self.as_ptr(), index.as_raw(), data.cast());
        }
    }

    #[corresponds(SSL_CTX_set_ex_data)]
    unsafe fn replace_ex_data<T>(&mut self, index: Index<SslContext, T>, data: T) -> Option<T> {
        unsafe {
            if let Some(old) = self.ex_data_mut(index) {
                return Some(mem::replace(old, data));
            }

            self.set_ex_data(index, data);

            None
        }
    }

    pub unsafe fn assume_x509(&mut self) {
        unsafe {
            self.replace_ex_data(*X509_FLAG_INDEX, true);
        }
    }

    #[must_use]
    pub fn has_x509_support(&self) -> bool {
        self.ex_data(*X509_FLAG_INDEX).copied().unwrap_or_default()
    }

    #[track_caller]
    fn check_x509(&self) {
        assert!(
            self.has_x509_support(),
            "This context is not configured for X.509 certificates"
        );
    }
}

#[derive(Debug)]
pub struct GetSessionPendingError;

pub struct CipherBits {
    pub secret: i32,

    pub algorithm: i32,
}

#[repr(transparent)]
pub struct ClientHello<'ssl>(&'ssl ffi::SSL_CLIENT_HELLO);

impl ClientHello<'_> {
    pub fn ssl_mut(&mut self) -> &mut SslRef {
        unsafe { SslRef::from_ptr_mut(self.0.ssl) }
    }

    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        unsafe { SslRef::from_ptr(self.0.ssl) }
    }

    #[must_use]
    pub fn servername(&self, type_: NameType) -> Option<&str> {
        self.ssl().servername(type_)
    }

    #[must_use]
    pub fn version_str(&self) -> &'static str {
        self.ssl().version_str()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.0.client_hello, self.0.client_hello_len) }
    }

    #[must_use]
    pub fn random(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.0.random, self.0.random_len) }
    }

    #[must_use]
    pub fn ciphers(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.0.cipher_suites, self.0.cipher_suites_len) }
    }
}

#[derive(Clone, Copy)]
pub struct SslCipher(&'static SslCipherRef);

impl SslCipher {
    #[corresponds(SSL_get_cipher_by_value)]
    #[must_use]
    pub fn from_value(value: u16) -> Option<Self> {
        unsafe {
            let ptr = ffi::SSL_get_cipher_by_value(value);
            if ptr.is_null() {
                None
            } else {
                Some(Self::from_ptr(ptr.cast_mut()))
            }
        }
    }
}

impl Stackable for SslCipher {
    type StackType = ffi::stack_st_SSL_CIPHER;
}

unsafe impl ForeignType for SslCipher {
    type CType = ffi::SSL_CIPHER;
    type Ref = SslCipherRef;

    #[inline]
    unsafe fn from_ptr(ptr: *mut ffi::SSL_CIPHER) -> SslCipher {
        SslCipher(unsafe { SslCipherRef::from_ptr(ptr) })
    }

    #[inline]
    fn as_ptr(&self) -> *mut ffi::SSL_CIPHER {
        self.0.as_ptr()
    }
}

impl Deref for SslCipher {
    type Target = SslCipherRef;

    fn deref(&self) -> &SslCipherRef {
        self.0
    }
}

pub struct SslCipherRef(Opaque);

unsafe impl Send for SslCipherRef {}
unsafe impl Sync for SslCipherRef {}

unsafe impl ForeignTypeRef for SslCipherRef {
    type CType = ffi::SSL_CIPHER;
}

impl SslCipherRef {
    #[corresponds(SSL_CIPHER_get_name)]
    #[must_use]
    pub fn name(&self) -> &'static str {
        unsafe {
            let ptr = ffi::SSL_CIPHER_get_name(self.as_ptr());
            CStr::from_ptr(ptr).to_str().unwrap()
        }
    }

    #[corresponds(SSL_CIPHER_get_version)]
    #[must_use]
    pub fn version(&self) -> &'static str {
        let version = unsafe {
            let ptr = ffi::SSL_CIPHER_get_version(self.as_ptr());
            CStr::from_ptr(ptr)
        };

        version.to_str().unwrap()
    }

    #[corresponds(SSL_CIPHER_get_bits)]
    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn bits(&self) -> CipherBits {
        unsafe {
            let mut algo_bits = 0;
            let secret_bits = ffi::SSL_CIPHER_get_bits(self.as_ptr(), &mut algo_bits);
            CipherBits {
                secret: secret_bits.into(),
                algorithm: algo_bits.into(),
            }
        }
    }

    #[corresponds(SSL_CIPHER_description)]
    #[must_use]
    pub fn description(&self) -> String {
        unsafe {
            let mut buf = [0; 128];
            let ptr = ffi::SSL_CIPHER_description(self.as_ptr(), buf.as_mut_ptr(), 128);
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::SSL_SESSION;
    fn drop = ffi::SSL_SESSION_free;

    pub struct SslSession;
}

impl Clone for SslSession {
    fn clone(&self) -> SslSession {
        SslSessionRef::to_owned(self)
    }
}

impl SslSession {
    from_der! {
        #[corresponds(d2i_SSL_SESSION)]
        from_der,
        SslSession,
        ffi::d2i_SSL_SESSION,
        ::libc::c_long
    }
}

impl ToOwned for SslSessionRef {
    type Owned = SslSession;

    fn to_owned(&self) -> SslSession {
        unsafe {
            SSL_SESSION_up_ref(self.as_ptr());
            SslSession(NonNull::new_unchecked(self.as_ptr()))
        }
    }
}

impl SslSessionRef {
    #[corresponds(SSL_SESSION_get_id)]
    #[must_use]
    pub fn id(&self) -> &[u8] {
        unsafe {
            let mut len = 0;
            let p = ffi::SSL_SESSION_get_id(self.as_ptr(), &mut len);
            slice::from_raw_parts(p, len as usize)
        }
    }

    #[corresponds(SSL_SESSION_get_time)]
    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn time(&self) -> u64 {
        unsafe { ffi::SSL_SESSION_get_time(self.as_ptr()) }
    }

    #[corresponds(SSL_SESSION_get_timeout)]
    #[allow(clippy::useless_conversion)]
    #[must_use]
    pub fn timeout(&self) -> u32 {
        unsafe { ffi::SSL_SESSION_get_timeout(self.as_ptr()) }
    }

    #[corresponds(SSL_SESSION_get_protocol_version)]
    #[must_use]
    pub fn protocol_version(&self) -> SslVersion {
        unsafe {
            let version = ffi::SSL_SESSION_get_protocol_version(self.as_ptr());
            SslVersion(version)
        }
    }

    to_der! {
        #[corresponds(i2d_SSL_SESSION)]
        to_der,
        ffi::i2d_SSL_SESSION
    }
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::SSL;
    fn drop = ffi::SSL_free;

    pub struct Ssl;
}

impl fmt::Debug for Ssl {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, fmt)
    }
}

impl Ssl {
    #[corresponds(SSL_get_ex_new_index)]
    pub fn new_ex_index<T>() -> Result<Index<Ssl, T>, ErrorStack>
    where
        T: 'static + Sync + Send,
    {
        unsafe {
            ffi::init();
            let idx = cvt_n(get_new_ssl_idx(Some(free_data_box::<T>)))?;
            Ok(Index::from_raw(idx))
        }
    }

    fn cached_ex_index<T>() -> Index<Ssl, T>
    where
        T: 'static + Sync + Send,
    {
        unsafe {
            let idx = *SSL_INDEXES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(TypeId::of::<T>())
                .or_insert_with(|| Ssl::new_ex_index::<T>().unwrap().as_raw());
            Index::from_raw(idx)
        }
    }

    #[corresponds(SSL_new)]
    pub fn new(ctx: &SslContextRef) -> Result<Ssl, ErrorStack> {
        unsafe {
            let ptr = cvt_p(ffi::SSL_new(ctx.as_ptr()))?;
            let mut ssl = Ssl::from_ptr(ptr);
            ssl.set_ex_data(*SESSION_CTX_INDEX, ctx.to_owned());

            Ok(ssl)
        }
    }

    pub fn setup_connect<S>(self, stream: S) -> MidHandshakeSslStream<S>
    where
        S: Read + Write,
    {
        SslStreamBuilder::new(self, stream).setup_connect()
    }

    pub fn connect<S>(self, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        self.setup_connect(stream).handshake()
    }

    pub fn setup_accept<S>(self, stream: S) -> MidHandshakeSslStream<S>
    where
        S: Read + Write,
    {
        SslStreamBuilder::new(self, stream).setup_accept()
    }

    pub fn accept<S>(self, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
    where
        S: Read + Write,
    {
        self.setup_accept(stream).handshake()
    }
}

impl fmt::Debug for SslRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let mut builder = fmt.debug_struct("Ssl");

        builder.field("state", &self.state_string_long());

        if self.ssl_context().has_x509_support() {
            builder.field("verify_result", &self.verify_result());
        }

        builder.field("verify_result", &self.verify_result());

        builder.finish()
    }
}

impl SslRef {
    fn get_raw_rbio(&self) -> *mut ffi::BIO {
        unsafe { ffi::SSL_get_rbio(self.as_ptr()) }
    }

    #[corresponds(SSL_set_options)]
    pub fn set_options(&mut self, option: SslOptions) -> SslOptions {
        let bits = unsafe { ffi::SSL_set_options(self.as_ptr(), option.bits()) };
        SslOptions::from_bits_retain(bits)
    }

    #[corresponds(SSL_set1_curves_list)]
    pub fn set_curves_list(&mut self, curves: &str) -> Result<(), ErrorStack> {
        let curves = CString::new(curves).map_err(ErrorStack::internal_error)?;
        unsafe { cvt_0i(ffi::SSL_set1_curves_list(self.as_ptr(), curves.as_ptr())).map(|_| ()) }
    }

    #[corresponds(SSL_get_curve_id)]
    #[must_use]
    pub fn curve(&self) -> Option<u16> {
        let curve_id = unsafe { ffi::SSL_get_curve_id(self.as_ptr()) };
        if curve_id == 0 {
            return None;
        }
        Some(curve_id)
    }

    #[corresponds(SSL_get_curve_name)]
    #[must_use]
    pub fn curve_name(&self) -> Option<&'static str> {
        let curve_id = self.curve()?;

        unsafe {
            let ptr = ffi::SSL_get_curve_name(curve_id);
            if ptr.is_null() {
                return None;
            }

            CStr::from_ptr(ptr).to_str().ok()
        }
    }

    #[corresponds(SSL_get_error)]
    #[must_use]
    pub fn error_code(&self, ret: c_int) -> ErrorCode {
        unsafe { ErrorCode::from_raw(ffi::SSL_get_error(self.as_ptr(), ret)) }
    }

    #[corresponds(SSL_set_verify)]
    pub fn set_verify(&mut self, mode: SslVerifyMode) {
        self.ssl_context().check_x509();

        unsafe { ffi::SSL_set_verify(self.as_ptr(), c_int::from(mode.bits()), None) }
    }

    #[corresponds(SSL_set_verify)]
    pub fn set_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(bool, &mut X509StoreContextRef) -> bool + 'static + Sync + Send,
    {
        self.ssl_context().check_x509();

        unsafe {
            self.replace_ex_data(Ssl::cached_ex_index(), Arc::new(callback));
            ffi::SSL_set_verify(
                self.as_ptr(),
                c_int::from(mode.bits()),
                Some(ssl_raw_verify::<F>),
            );
        }
    }

    #[corresponds(SSL_set_custom_verify)]
    pub fn set_custom_verify_callback<F>(&mut self, mode: SslVerifyMode, callback: F)
    where
        F: Fn(&mut SslRef) -> Result<(), SslVerifyError> + 'static + Sync + Send,
    {
        self.ssl_context().check_x509();

        unsafe {
            self.replace_ex_data(Ssl::cached_ex_index(), Arc::new(callback));
            ffi::SSL_set_custom_verify(
                self.as_ptr(),
                c_int::from(mode.bits()),
                Some(ssl_raw_custom_verify::<F>),
            );
        }
    }

    #[corresponds(SSL_set_permute_extensions)]
    pub fn set_permute_extensions(&mut self, enabled: bool) {
        unsafe { ffi::SSL_set_permute_extensions(self.as_ptr(), enabled as _) }
    }

    #[corresponds(SSL_set_alpn_protos)]
    pub fn set_alpn_protos(&mut self, protocols: &[u8]) -> Result<(), ErrorStack> {
        unsafe {
            let r = ffi::SSL_set_alpn_protos(
                self.as_ptr(),
                protocols.as_ptr(),
                try_int(protocols.len())?,
            );
            if r == 0 {
                Ok(())
            } else {
                Err(ErrorStack::get())
            }
        }
    }

    #[corresponds(SSL_get_ciphers)]
    #[must_use]
    pub fn ciphers(&self) -> &StackRef<SslCipher> {
        unsafe {
            let cipher_list = ffi::SSL_get_ciphers(self.as_ptr());
            StackRef::from_ptr(cipher_list)
        }
    }

    #[corresponds(SSL_get_current_cipher)]
    #[must_use]
    pub fn current_cipher(&self) -> Option<&SslCipherRef> {
        unsafe {
            let ptr = ffi::SSL_get_current_cipher(self.as_ptr());

            if ptr.is_null() {
                None
            } else {
                Some(SslCipherRef::from_ptr(ptr.cast_mut()))
            }
        }
    }

    #[corresponds(SSL_state_string_long)]
    #[must_use]
    pub fn state_string_long(&self) -> &'static str {
        let state = unsafe {
            let ptr = ffi::SSL_state_string_long(self.as_ptr());
            CStr::from_ptr(ptr)
        };

        state.to_str().unwrap_or_default()
    }

    #[corresponds(SSL_set_tlsext_host_name)]
    pub fn set_hostname(&mut self, hostname: &str) -> Result<(), ErrorStack> {
        let cstr = CString::new(hostname).map_err(ErrorStack::internal_error)?;
        unsafe { cvt(ffi::SSL_set_tlsext_host_name(self.as_ptr(), cstr.as_ptr())) }
    }

    #[corresponds(SSL_get_peer_certificate)]
    #[must_use]
    pub fn peer_certificate(&self) -> Option<X509> {
        self.ssl_context().check_x509();

        unsafe {
            let ptr = ffi::SSL_get_peer_certificate(self.as_ptr());
            if ptr.is_null() {
                None
            } else {
                Some(X509::from_ptr(ptr))
            }
        }
    }

    #[corresponds(SSL_get_peer_cert_chain)]
    #[must_use]
    pub fn peer_cert_chain(&self) -> Option<&StackRef<X509>> {
        self.ssl_context().check_x509();

        unsafe {
            let ptr = ffi::SSL_get_peer_cert_chain(self.as_ptr());
            if ptr.is_null() {
                None
            } else {
                Some(StackRef::from_ptr(ptr))
            }
        }
    }

    #[corresponds(SSL_get_certificate)]
    #[must_use]
    pub fn certificate(&self) -> Option<&X509Ref> {
        self.ssl_context().check_x509();

        unsafe {
            let ptr = ffi::SSL_get_certificate(self.as_ptr());
            if ptr.is_null() {
                None
            } else {
                Some(X509Ref::from_ptr(ptr))
            }
        }
    }

    #[deprecated(since = "0.10.5", note = "renamed to `version_str`")]
    #[must_use]
    pub fn version(&self) -> &str {
        self.version_str()
    }

    #[corresponds(SSL_get_version)]
    #[must_use]
    pub fn version_str(&self) -> &'static str {
        let version = unsafe {
            let ptr = ffi::SSL_get_version(self.as_ptr());
            CStr::from_ptr(ptr)
        };

        version.to_str().unwrap()
    }

    #[corresponds(SSL_set_min_proto_version)]
    pub fn set_min_proto_version(&mut self, version: Option<SslVersion>) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_set_min_proto_version(
                self.as_ptr(),
                version.map_or(0, |v| v.0 as _),
            ))
        }
    }

    #[corresponds(SSL_set_max_proto_version)]
    pub fn set_max_proto_version(&mut self, version: Option<SslVersion>) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_set_max_proto_version(
                self.as_ptr(),
                version.map_or(0, |v| v.0 as _),
            ))
        }
    }

    #[corresponds(SSL_get0_alpn_selected)]
    #[must_use]
    pub fn selected_alpn_protocol(&self) -> Option<&[u8]> {
        unsafe {
            let mut data: *const c_uchar = ptr::null();
            let mut len: c_uint = 0;
            ffi::SSL_get0_alpn_selected(self.as_ptr(), &mut data, &mut len);

            if data.is_null() {
                None
            } else {
                Some(slice::from_raw_parts(data, len as usize))
            }
        }
    }

    #[corresponds(SSL_pending)]
    #[must_use]
    pub fn pending(&self) -> usize {
        unsafe { ffi::SSL_pending(self.as_ptr()) as usize }
    }

    #[corresponds(SSL_get_servername)]
    #[must_use]
    pub fn servername(&self, type_: NameType) -> Option<&str> {
        self.servername_raw(type_)
            .and_then(|b| str::from_utf8(b).ok())
    }

    #[corresponds(SSL_get_servername)]
    #[must_use]
    pub fn servername_raw(&self, type_: NameType) -> Option<&[u8]> {
        unsafe {
            let name = ffi::SSL_get_servername(self.as_ptr(), type_.0);
            if name.is_null() {
                None
            } else {
                Some(CStr::from_ptr(name).to_bytes())
            }
        }
    }

    #[corresponds(SSL_get_SSL_CTX)]
    #[must_use]
    pub fn ssl_context(&self) -> &SslContextRef {
        unsafe {
            let ssl_ctx = ffi::SSL_get_SSL_CTX(self.as_ptr());
            SslContextRef::from_ptr(ssl_ctx)
        }
    }

    #[corresponds(SSL_get0_param)]
    pub fn verify_param_mut(&mut self) -> &mut X509VerifyParamRef {
        self.ssl_context().check_x509();

        unsafe { X509VerifyParamRef::from_ptr_mut(ffi::SSL_get0_param(self.as_ptr())) }
    }

    pub fn param_mut(&mut self) -> &mut X509VerifyParamRef {
        self.verify_param_mut()
    }

    #[corresponds(SSL_get_verify_result)]
    pub fn verify_result(&self) -> X509VerifyResult {
        self.ssl_context().check_x509();

        unsafe { X509VerifyError::from_raw(ffi::SSL_get_verify_result(self.as_ptr()) as c_int) }
    }

    #[corresponds(SSL_get_session)]
    #[must_use]
    pub fn session(&self) -> Option<&SslSessionRef> {
        unsafe {
            let p = ffi::SSL_get_session(self.as_ptr());
            if p.is_null() {
                None
            } else {
                Some(SslSessionRef::from_ptr(p))
            }
        }
    }

    #[corresponds(SSL_set_session)]
    pub unsafe fn set_session(&mut self, session: &SslSessionRef) -> Result<(), ErrorStack> {
        unsafe { cvt(ffi::SSL_set_session(self.as_ptr(), session.as_ptr())) }
    }

    #[corresponds(SSL_session_reused)]
    #[must_use]
    pub fn session_reused(&self) -> bool {
        unsafe { ffi::SSL_session_reused(self.as_ptr()) != 0 }
    }

    #[corresponds(SSL_is_server)]
    #[must_use]
    pub fn is_server(&self) -> bool {
        unsafe { SSL_is_server(self.as_ptr()) != 0 }
    }

    #[corresponds(SSL_set_ex_data)]
    pub fn set_ex_data<T>(&mut self, index: Index<Ssl, T>, data: T) {
        if let Some(old) = self.ex_data_mut(index) {
            *old = data;
            return;
        }

        unsafe {
            let data = Box::into_raw(Box::new(data));
            ffi::SSL_set_ex_data(self.as_ptr(), index.as_raw(), data.cast());
        }
    }

    #[corresponds(SSL_set_ex_data)]
    pub fn replace_ex_data<T>(&mut self, index: Index<Ssl, T>, data: T) -> Option<T> {
        if let Some(old) = self.ex_data_mut(index) {
            return Some(mem::replace(old, data));
        }

        self.set_ex_data(index, data);

        None
    }

    #[corresponds(SSL_get_ex_data)]
    #[must_use]
    pub fn ex_data<T>(&self, index: Index<Ssl, T>) -> Option<&T> {
        unsafe {
            let data = ffi::SSL_get_ex_data(self.as_ptr(), index.as_raw());
            if data.is_null() {
                None
            } else {
                Some(&*(data as *const T))
            }
        }
    }

    #[corresponds(SSL_get_ex_data)]
    pub fn ex_data_mut<T>(&mut self, index: Index<Ssl, T>) -> Option<&mut T> {
        unsafe {
            ffi::SSL_get_ex_data(self.as_ptr(), index.as_raw())
                .cast::<T>()
                .as_mut()
        }
    }

    #[corresponds(SSL_get_finished)]
    pub fn finished(&self, buf: &mut [u8]) -> usize {
        unsafe { ffi::SSL_get_finished(self.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) }
    }

    #[corresponds(SSL_get_peer_finished)]
    pub fn peer_finished(&self, buf: &mut [u8]) -> usize {
        unsafe { ffi::SSL_get_peer_finished(self.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) }
    }

    #[corresponds(SSL_use_certificate)]
    pub fn set_certificate(&mut self, cert: &X509Ref) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_use_certificate(self.as_ptr(), cert.as_ptr()))?;
        }

        Ok(())
    }

    #[corresponds(SSL_use_PrivateKey)]
    pub fn set_private_key<T>(&mut self, key: &PKeyRef<T>) -> Result<(), ErrorStack>
    where
        T: HasPrivate,
    {
        unsafe { cvt(ffi::SSL_use_PrivateKey(self.as_ptr(), key.as_ptr())) }
    }

    #[corresponds(SSL_set1_requested_trust_anchors)]
    pub fn set_requested_trust_anchors(&mut self, ids: &[u8]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_set1_requested_trust_anchors(
                self.as_ptr(),
                ids.as_ptr(),
                ids.len(),
            ))
        }
    }

    #[corresponds(SSL_set_tls12_extensions)]
    pub fn set_tls12_extensions(&mut self, enabled: bool) {
        unsafe { ffi::SSL_set_tls12_extensions(self.as_ptr(), c_int::from(enabled)) }
    }

    #[corresponds(SSL_add_application_settings)]
    pub fn add_application_settings(&mut self, alps: &[u8]) -> Result<(), ErrorStack> {
        unsafe {
            cvt(ffi::SSL_add_application_settings(
                self.as_ptr(),
                alps.as_ptr(),
                alps.len(),
                std::ptr::null(),
                0,
            ))
        }
    }

    #[corresponds(SSL_set_alps_use_new_codepoint)]
    pub fn set_alps_use_new_codepoint(&mut self, enable: bool) {
        unsafe { ffi::SSL_set_alps_use_new_codepoint(self.as_ptr(), enable.into()) }
    }

    #[corresponds(SSL_set_enable_ech_grease)]
    pub fn set_enable_ech_grease(&self, enable: bool) {
        let enable = if enable { 1 } else { 0 };

        unsafe {
            ffi::SSL_set_enable_ech_grease(self.as_ptr(), enable);
        }
    }
}

#[derive(Debug)]
pub struct MidHandshakeSslStream<S> {
    stream: SslStream<S>,
    error: Error,
}

impl<S> MidHandshakeSslStream<S> {
    #[must_use]
    pub fn get_ref(&self) -> &S {
        self.stream.get_ref()
    }

    pub fn get_mut(&mut self) -> &mut S {
        self.stream.get_mut()
    }

    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        self.stream.ssl()
    }

    pub fn ssl_mut(&mut self) -> &mut SslRef {
        self.stream.ssl_mut()
    }

    #[must_use]
    pub fn error(&self) -> &Error {
        &self.error
    }

    #[must_use]
    pub fn into_source_stream(self) -> S {
        self.stream.into_inner()
    }

    #[must_use]
    pub fn into_parts(self) -> (Error, S) {
        (self.error, self.stream.into_inner())
    }

    #[corresponds(SSL_do_handshake)]
    pub fn handshake(mut self) -> Result<SslStream<S>, HandshakeError<S>> {
        let ret = unsafe { ffi::SSL_do_handshake(self.stream.ssl.as_ptr()) };
        if ret > 0 {
            Ok(self.stream)
        } else {
            self.error = self.stream.make_error(ret);
            Err(if self.error.would_block() {
                HandshakeError::WouldBlock(self)
            } else {
                HandshakeError::Failure(self)
            })
        }
    }
}

pub struct SslStream<S> {
    ssl: ManuallyDrop<Ssl>,
    method: ManuallyDrop<BioMethod>,
    _p: PhantomData<S>,
}

impl<S> Drop for SslStream<S> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.ssl);
            ManuallyDrop::drop(&mut self.method);
        }
    }
}

impl<S> fmt::Debug for SslStream<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("SslStream")
            .field("stream", &self.get_ref())
            .field("ssl", &self.ssl())
            .finish()
    }
}

impl<S: Read + Write> SslStream<S> {
    pub fn new(ssl: Ssl, stream: S) -> Result<Self, ErrorStack> {
        let (bio, method) = bio::new(stream)?;

        unsafe {
            ffi::SSL_set_bio(ssl.as_ptr(), bio, bio);
        }

        Ok(SslStream {
            ssl: ManuallyDrop::new(ssl),
            method: ManuallyDrop::new(method),
            _p: PhantomData,
        })
    }

    pub unsafe fn from_raw_parts(ssl: *mut ffi::SSL, stream: S) -> Self {
        let ssl = unsafe { Ssl::from_ptr(ssl) };
        Self::new(ssl, stream).unwrap()
    }

    pub fn read_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> io::Result<usize> {
        loop {
            match self.ssl_read_uninit(buf) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.code() == ErrorCode::ZERO_RETURN => return Ok(0),
                Err(ref e) if e.code() == ErrorCode::SYSCALL && e.io_error().is_none() => {
                    return Ok(0);
                }
                Err(ref e) if e.code() == ErrorCode::WANT_READ && e.io_error().is_none() => {}
                Err(e) => {
                    return Err(e.into_io_error().unwrap_or_else(io::Error::other));
                }
            }
        }
    }

    pub fn ssl_read_uninit(&mut self, buf: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let len = usize::min(c_int::MAX as usize, buf.len()) as c_int;
        let ret = unsafe { ffi::SSL_read(self.ssl().as_ptr(), buf.as_mut_ptr().cast(), len) };
        if ret > 0 {
            Ok(ret as usize)
        } else {
            Err(self.make_error(ret))
        }
    }

    #[corresponds(SSL_write)]
    pub fn ssl_write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let len = usize::min(c_int::MAX as usize, buf.len()) as c_int;
        let ret = unsafe { ffi::SSL_write(self.ssl().as_ptr(), buf.as_ptr().cast(), len) };
        if ret > 0 {
            Ok(ret as usize)
        } else {
            Err(self.make_error(ret))
        }
    }

    #[corresponds(SSL_shutdown)]
    pub fn shutdown(&mut self) -> Result<ShutdownResult, Error> {
        match unsafe { ffi::SSL_shutdown(self.ssl.as_ptr()) } {
            0 => Ok(ShutdownResult::Sent),
            1 => Ok(ShutdownResult::Received),
            n => Err(self.make_error(n)),
        }
    }

    #[corresponds(SSL_connect)]
    pub fn connect(&mut self) -> Result<(), Error> {
        let ret = unsafe { ffi::SSL_connect(self.ssl.as_ptr()) };
        if ret > 0 {
            Ok(())
        } else {
            Err(self.make_error(ret))
        }
    }

    #[corresponds(SSL_accept)]
    pub fn accept(&mut self) -> Result<(), Error> {
        let ret = unsafe { ffi::SSL_accept(self.ssl.as_ptr()) };
        if ret > 0 {
            Ok(())
        } else {
            Err(self.make_error(ret))
        }
    }

    #[corresponds(SSL_do_handshake)]
    pub fn do_handshake(&mut self) -> Result<(), Error> {
        let ret = unsafe { ffi::SSL_do_handshake(self.ssl.as_ptr()) };
        if ret > 0 {
            Ok(())
        } else {
            Err(self.make_error(ret))
        }
    }
}

impl<S> SslStream<S> {
    fn make_error(&mut self, ret: c_int) -> Error {
        self.check_panic();

        let code = self.ssl.error_code(ret);

        let cause = match code {
            ErrorCode::SSL => Some(InnerError::Ssl(ErrorStack::get())),
            ErrorCode::SYSCALL => {
                let errs = ErrorStack::get();
                if errs.errors().is_empty() {
                    self.get_bio_error().map(InnerError::Io)
                } else {
                    Some(InnerError::Ssl(errs))
                }
            }
            ErrorCode::ZERO_RETURN => None,
            ErrorCode::WANT_READ | ErrorCode::WANT_WRITE => {
                self.get_bio_error().map(InnerError::Io)
            }
            _ => None,
        };

        Error { code, cause }
    }

    fn check_panic(&mut self) {
        if let Some(err) = unsafe { bio::take_panic::<S>(self.ssl.get_raw_rbio()) } {
            resume_unwind(err)
        }
    }

    fn get_bio_error(&mut self) -> Option<io::Error> {
        unsafe { bio::take_error::<S>(self.ssl.get_raw_rbio()) }
    }

    #[must_use]
    pub fn into_inner(self) -> S {
        unsafe { bio::take_stream::<S>(self.ssl.get_raw_rbio()) }
    }

    #[must_use]
    pub fn get_ref(&self) -> &S {
        unsafe {
            let bio = self.ssl.get_raw_rbio();
            bio::get_ref(bio)
        }
    }

    pub fn get_mut(&mut self) -> &mut S {
        unsafe {
            let bio = self.ssl.get_raw_rbio();
            bio::get_mut(bio)
        }
    }

    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        &self.ssl
    }

    pub fn ssl_mut(&mut self) -> &mut SslRef {
        &mut self.ssl
    }
}

impl<S: Read + Write> Read for SslStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `read_uninit` does not de-initialize the buffer
        unsafe {
            self.read_uninit(slice::from_raw_parts_mut(
                buf.as_mut_ptr().cast::<MaybeUninit<u8>>(),
                buf.len(),
            ))
        }
    }
}

impl<S: Read + Write> Write for SslStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match self.ssl_write(buf) {
                Ok(n) => return Ok(n),
                Err(ref e) if e.code() == ErrorCode::WANT_READ && e.io_error().is_none() => {}
                Err(e) => {
                    return Err(e.into_io_error().unwrap_or_else(io::Error::other));
                }
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.get_mut().flush()
    }
}

pub struct SslStreamBuilder<S> {
    inner: SslStream<S>,
}

impl<S> SslStreamBuilder<S>
where
    S: Read + Write,
{
    pub fn new(ssl: Ssl, stream: S) -> Self {
        Self {
            inner: SslStream::new(ssl, stream).unwrap(),
        }
    }

    #[corresponds(SSL_set_connect_state)]
    pub fn set_connect_state(&mut self) {
        unsafe { ffi::SSL_set_connect_state(self.inner.ssl.as_ptr()) }
    }

    #[corresponds(SSL_set_accept_state)]
    pub fn set_accept_state(&mut self) {
        unsafe { ffi::SSL_set_accept_state(self.inner.ssl.as_ptr()) }
    }

    #[must_use]
    pub fn setup_connect(mut self) -> MidHandshakeSslStream<S> {
        self.set_connect_state();

        MidHandshakeSslStream {
            stream: self.inner,
            error: Error {
                code: ErrorCode::WANT_WRITE,
                cause: Some(InnerError::Io(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "connect handshake has not started yet",
                ))),
            },
        }
    }

    pub fn connect(self) -> Result<SslStream<S>, HandshakeError<S>> {
        self.setup_connect().handshake()
    }

    #[must_use]
    pub fn setup_accept(mut self) -> MidHandshakeSslStream<S> {
        self.set_accept_state();

        MidHandshakeSslStream {
            stream: self.inner,
            error: Error {
                code: ErrorCode::WANT_READ,
                cause: Some(InnerError::Io(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "accept handshake has not started yet",
                ))),
            },
        }
    }

    pub fn accept(self) -> Result<SslStream<S>, HandshakeError<S>> {
        self.setup_accept().handshake()
    }

    #[corresponds(SSL_do_handshake)]
    pub fn handshake(self) -> Result<SslStream<S>, HandshakeError<S>> {
        let mut stream = self.inner;
        let ret = unsafe { ffi::SSL_do_handshake(stream.ssl.as_ptr()) };
        if ret > 0 {
            Ok(stream)
        } else {
            let error = stream.make_error(ret);
            Err(if error.would_block() {
                HandshakeError::WouldBlock(MidHandshakeSslStream { stream, error })
            } else {
                HandshakeError::Failure(MidHandshakeSslStream { stream, error })
            })
        }
    }
}

impl<S> SslStreamBuilder<S> {
    #[must_use]
    pub fn get_ref(&self) -> &S {
        unsafe {
            let bio = self.inner.ssl.get_raw_rbio();
            bio::get_ref(bio)
        }
    }

    pub fn get_mut(&mut self) -> &mut S {
        unsafe {
            let bio = self.inner.ssl.get_raw_rbio();
            bio::get_mut(bio)
        }
    }

    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        &self.inner.ssl
    }

    pub fn ssl_mut(&mut self) -> &mut SslRef {
        &mut self.inner.ssl
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShutdownResult {
    Sent,

    Received,
}

bitflags! {
    #[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord, Hash)]
    pub struct ShutdownState: c_int {
        const SENT = ffi::SSL_SENT_SHUTDOWN;
        const RECEIVED = ffi::SSL_RECEIVED_SHUTDOWN;
    }
}

pub trait PrivateKeyMethod: Send + Sync + 'static {
    fn sign(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        signature_algorithm: SslSignatureAlgorithm,
        output: &mut [u8],
    ) -> Result<usize, PrivateKeyMethodError>;

    fn decrypt(
        &self,
        ssl: &mut SslRef,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<usize, PrivateKeyMethodError>;

    fn complete(&self, ssl: &mut SslRef, output: &mut [u8])
        -> Result<usize, PrivateKeyMethodError>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PrivateKeyMethodError(ffi::ssl_private_key_result_t);

impl PrivateKeyMethodError {
    pub const FAILURE: Self = Self(ffi::ssl_private_key_result_t::ssl_private_key_failure);

    pub const RETRY: Self = Self(ffi::ssl_private_key_result_t::ssl_private_key_retry);
}

pub trait CertificateCompressor: Send + Sync + 'static {
    const ALGORITHM: CertificateCompressionAlgorithm;

    const CAN_COMPRESS: bool;

    const CAN_DECOMPRESS: bool;

    #[allow(unused_variables)]
    fn compress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        Err(std::io::Error::other("not implemented"))
    }

    #[allow(unused_variables)]
    fn decompress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        Err(std::io::Error::other("not implemented"))
    }
}

use crate::ffi::{SSL_CTX_up_ref, SSL_SESSION_up_ref, SSL_is_server};

unsafe fn get_new_idx(f: ffi::CRYPTO_EX_free) -> c_int {
    unsafe { ffi::SSL_CTX_get_ex_new_index(0, ptr::null_mut(), ptr::null_mut(), None, f) }
}

unsafe fn get_new_ssl_idx(f: ffi::CRYPTO_EX_free) -> c_int {
    unsafe { ffi::SSL_get_ex_new_index(0, ptr::null_mut(), ptr::null_mut(), None, f) }
}

fn path_to_cstring(path: &Path) -> Result<CString, ErrorStack> {
    CString::new(path.as_os_str().as_encoded_bytes()).map_err(ErrorStack::internal_error)
}
