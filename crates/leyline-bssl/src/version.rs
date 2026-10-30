use std::ffi::CStr;

use crate::ffi::{
    OpenSSL_version, OpenSSL_version_num, OPENSSL_DIR, OPENSSL_PLATFORM, OPENSSL_VERSION,
};

#[must_use]
pub fn number() -> i64 {
    unsafe { OpenSSL_version_num() as i64 }
}

#[must_use]
pub fn version() -> &'static str {
    unsafe {
        CStr::from_ptr(OpenSSL_version(OPENSSL_VERSION))
            .to_str()
            .unwrap()
    }
}

#[must_use]
pub fn platform() -> &'static str {
    unsafe {
        CStr::from_ptr(OpenSSL_version(OPENSSL_PLATFORM))
            .to_str()
            .unwrap()
    }
}

#[must_use]
pub fn dir() -> &'static str {
    unsafe {
        CStr::from_ptr(OpenSSL_version(OPENSSL_DIR))
            .to_str()
            .unwrap()
    }
}
