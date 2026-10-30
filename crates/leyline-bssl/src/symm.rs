use crate::ffi;
use openssl_macros::corresponds;
use std::cmp;
use std::ffi::c_int;
use std::ptr;

use crate::error::ErrorStack;
use crate::nid::Nid;
use crate::{cvt, cvt_p, try_int};

#[derive(Copy, Clone)]
pub enum Mode {
    Encrypt,
    Decrypt,
}

foreign_type_and_impl_send_sync! {
    type CType = ffi::EVP_CIPHER_CTX;
    fn drop = ffi::EVP_CIPHER_CTX_free;

    pub struct CipherCtx;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Cipher(*const ffi::EVP_CIPHER);

impl Cipher {
    #[must_use]
    pub unsafe fn from_ptr(ptr: *const ffi::EVP_CIPHER) -> Cipher {
        Cipher(ptr)
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn as_ptr(&self) -> *const ffi::EVP_CIPHER {
        self.0
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn key_len(&self) -> usize {
        unsafe { EVP_CIPHER_key_length(self.0) as usize }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn iv_len(&self) -> Option<usize> {
        unsafe {
            let len = EVP_CIPHER_iv_length(self.0) as usize;
            if len == 0 {
                None
            } else {
                Some(len)
            }
        }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn block_size(&self) -> usize {
        unsafe { EVP_CIPHER_block_size(self.0) as usize }
    }

    #[corresponds(EVP_CIPHER_nid)]
    pub fn nid(&self) -> Nid {
        ffi::init();
        let nid = unsafe { ffi::EVP_CIPHER_nid(self.as_ptr()) };
        Nid::from_raw(nid)
    }
}

unsafe impl Sync for Cipher {}
unsafe impl Send for Cipher {}

pub struct Crypter {
    ctx: *mut ffi::EVP_CIPHER_CTX,
    block_size: usize,
}

unsafe impl Sync for Crypter {}
unsafe impl Send for Crypter {}

impl Crypter {
    pub fn new(
        t: Cipher,
        mode: Mode,
        key: &[u8],
        iv: Option<&[u8]>,
    ) -> Result<Crypter, ErrorStack> {
        ffi::init();

        unsafe {
            let ctx = cvt_p(ffi::EVP_CIPHER_CTX_new())?;
            let crypter = Crypter {
                ctx,
                block_size: t.block_size(),
            };

            let mode = match mode {
                Mode::Encrypt => 1,
                Mode::Decrypt => 0,
            };

            cvt(ffi::EVP_CipherInit_ex(
                crypter.ctx,
                t.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                mode,
            ))?;

            cvt(ffi::EVP_CIPHER_CTX_set_key_length(
                crypter.ctx,
                try_int(key.len())?,
            ))?;

            let iv = match (iv, t.iv_len()) {
                (Some(iv), Some(len)) => {
                    if iv.len() != len {
                        cvt(ffi::EVP_CIPHER_CTX_ctrl(
                            crypter.ctx,
                            ffi::EVP_CTRL_GCM_SET_IVLEN,
                            try_int(iv.len())?,
                            ptr::null_mut(),
                        ))?;
                    }
                    iv.as_ptr().cast_mut()
                }
                (Some(_) | None, None) => ptr::null_mut(),
                (None, Some(_)) => panic!("an IV is required for this cipher"),
            };
            cvt(ffi::EVP_CipherInit_ex(
                crypter.ctx,
                ptr::null(),
                ptr::null_mut(),
                key.as_ptr().cast_mut(),
                iv,
                mode,
            ))?;

            Ok(crypter)
        }
    }

    pub fn pad(&mut self, padding: bool) {
        unsafe {
            ffi::EVP_CIPHER_CTX_set_padding(self.ctx, c_int::from(padding));
        }
    }

    pub fn update(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, ErrorStack> {
        unsafe {
            let block_size = if self.block_size > 1 {
                self.block_size
            } else {
                0
            };
            assert!(output.len() >= input.len() + block_size);
            let mut outl = try_int(output.len())?;

            cvt(ffi::EVP_CipherUpdate(
                self.ctx,
                output.as_mut_ptr(),
                &mut outl,
                input.as_ptr(),
                try_int(input.len())?,
            ))?;

            Ok(outl as usize)
        }
    }

    pub fn finalize(&mut self, output: &mut [u8]) -> Result<usize, ErrorStack> {
        unsafe {
            if self.block_size > 1 {
                assert!(output.len() >= self.block_size);
            }
            let mut outl = cmp::min(output.len(), c_int::MAX as usize) as c_int;

            cvt(ffi::EVP_CipherFinal_ex(
                self.ctx,
                output.as_mut_ptr(),
                &mut outl,
            ))?;

            Ok(outl as usize)
        }
    }
}

impl Drop for Crypter {
    fn drop(&mut self) {
        unsafe {
            ffi::EVP_CIPHER_CTX_free(self.ctx);
        }
    }
}

pub fn encrypt(
    t: Cipher,
    key: &[u8],
    iv: Option<&[u8]>,
    data: &[u8],
) -> Result<Vec<u8>, ErrorStack> {
    cipher(t, Mode::Encrypt, key, iv, data)
}

pub fn decrypt(
    t: Cipher,
    key: &[u8],
    iv: Option<&[u8]>,
    data: &[u8],
) -> Result<Vec<u8>, ErrorStack> {
    cipher(t, Mode::Decrypt, key, iv, data)
}

fn cipher(
    t: Cipher,
    mode: Mode,
    key: &[u8],
    iv: Option<&[u8]>,
    data: &[u8],
) -> Result<Vec<u8>, ErrorStack> {
    let mut c = Crypter::new(t, mode, key, iv)?;
    let mut out = vec![0; data.len() + t.block_size()];
    let count = c.update(data, &mut out)?;
    let rest = c.finalize(&mut out[count..])?;
    out.truncate(count + rest);
    Ok(out)
}

use crate::ffi::{EVP_CIPHER_block_size, EVP_CIPHER_iv_length, EVP_CIPHER_key_length};
