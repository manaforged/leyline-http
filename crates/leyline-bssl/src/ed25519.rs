use crate::ffi;

pub const PUBLIC_KEY_LEN: usize = ffi::ED25519_PUBLIC_KEY_LEN as usize;
pub const SIGNATURE_LEN: usize = ffi::ED25519_SIGNATURE_LEN as usize;

#[must_use]
pub fn verify(
    message: &[u8],
    signature: &[u8; SIGNATURE_LEN],
    public_key: &[u8; PUBLIC_KEY_LEN],
) -> bool {
    ffi::init();
    // SAFETY: the message pointer and length come from one slice, and both arrays have the lengths ED25519_verify reads.
    unsafe {
        ffi::ED25519_verify(
            message.as_ptr(),
            message.len(),
            signature.as_ptr(),
            public_key.as_ptr(),
        ) == 1
    }
}
