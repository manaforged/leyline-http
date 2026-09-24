#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate foreign_types;
extern crate leyline_bssl_sys as ffi;
extern crate libc;

#[cfg(test)]
extern crate hex;

use std::ffi::{c_int, c_long, c_void};
use std::num::NonZeroUsize;

#[doc(inline)]
pub use crate::ffi::init;

use crate::error::ErrorStack;

#[macro_use]
mod macros;

mod bio;
#[macro_use]
mod util;
pub mod asn1;
pub mod bn;
pub mod conf;
pub mod derive;
pub mod dh;
pub mod dsa;
pub mod ec;
pub mod error;
pub mod ex_data;
pub mod hash;
pub mod hmac;
pub mod hpke;
pub mod nid;
pub mod pkey;
pub mod rsa;
pub mod srtp;
pub mod ssl;
pub mod stack;
pub mod string;
pub mod symm;
pub mod version;
pub mod x509;

fn cvt_p<T>(r: *mut T) -> Result<*mut T, ErrorStack> {
    if r.is_null() {
        Err(ErrorStack::get())
    } else {
        Ok(r)
    }
}

fn cvt_0(r: usize) -> Result<(), ErrorStack> {
    if r == 0 {
        Err(ErrorStack::get())
    } else {
        Ok(())
    }
}

fn cvt_0i(r: c_int) -> Result<c_int, ErrorStack> {
    if r == 0 {
        Err(ErrorStack::get())
    } else {
        Ok(r)
    }
}

fn cvt(r: c_int) -> Result<(), ErrorStack> {
    if r <= 0 {
        Err(ErrorStack::get())
    } else {
        Ok(())
    }
}

fn cvt_nz(r: c_int) -> Result<NonZeroUsize, ErrorStack> {
    usize::try_from(r)
        .ok()
        .and_then(NonZeroUsize::new)
        .ok_or_else(ErrorStack::get)
}

fn cvt_n(r: c_int) -> Result<c_int, ErrorStack> {
    if r < 0 {
        Err(ErrorStack::get())
    } else {
        Ok(r)
    }
}

fn try_int<F, T>(from: F) -> Result<T, ErrorStack>
where
    F: TryInto<T> + Send + Sync + Copy + 'static,
    T: Send + Sync + Copy + 'static,
{
    from.try_into()
        .map_err(|_| ErrorStack::internal_error_str("int overflow"))
}

unsafe extern "C" fn free_data_box<T>(
    _parent: *mut c_void,
    ptr: *mut c_void,
    _ad: *mut ffi::CRYPTO_EX_DATA,
    _idx: c_int,
    _argl: c_long,
    _argp: *mut c_void,
) {
    if !ptr.is_null() {
        unsafe {
            drop(Box::<T>::from_raw(ptr.cast::<T>()));
        }
    }
}
