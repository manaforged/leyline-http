use std::ffi::CStr;

use crate::ffi::{
    OpenSSL_version, OpenSSL_version_num, OPENSSL_BUILT_ON, OPENSSL_CFLAGS, OPENSSL_DIR,
    OPENSSL_PLATFORM, OPENSSL_VERSION,
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
pub fn c_flags() -> &'static str {
    unsafe {
        CStr::from_ptr(OpenSSL_version(OPENSSL_CFLAGS))
            .to_str()
            .unwrap()
    }
}

#[must_use]
pub fn built_on() -> &'static str {
    unsafe {
        CStr::from_ptr(OpenSSL_version(OPENSSL_BUILT_ON))
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

#[test]
fn test_versions() {
    println!("Number: '{}'", number());
    println!("Version: '{}'", version());
    println!("C flags: '{}'", c_flags());
    println!("Built on: '{}'", built_on());
    println!("Platform: '{}'", platform());
    println!("Dir: '{}'", dir());

    assert!(number() > 0);
    assert!(version().starts_with("BoringSSL"));
    assert!(c_flags().starts_with("compiler:"));
    assert!(built_on().starts_with("built on:"));
    assert!(dir().starts_with("OPENSSLDIR:"));
}
