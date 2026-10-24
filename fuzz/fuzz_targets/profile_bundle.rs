#![no_main]

use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use leyline::profile::{BundleSource, ProfileBundle};
use leyline_bssl_sys::{ED25519_keypair_from_seed, ED25519_sign};
use libfuzzer_sys::fuzz_target;

const RFC8032_TEST1_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

static KEYPAIR: LazyLock<([u8; 32], [u8; 64])> = LazyLock::new(|| {
    let mut public = [0; 32];
    let mut private = [0; 64];
    // SAFETY: both output buffers have the lengths ED25519_keypair_from_seed writes, and the seed is 32 bytes.
    unsafe {
        ED25519_keypair_from_seed(
            public.as_mut_ptr(),
            private.as_mut_ptr(),
            RFC8032_TEST1_SEED.as_ptr(),
        );
    }
    (public, private)
});

fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut signature = [0; 64];
    // SAFETY: the signature buffer is 64 bytes, the payload pointer and length come from one slice, and the private key is 64 bytes.
    let signed = unsafe {
        ED25519_sign(
            signature.as_mut_ptr(),
            payload.as_ptr(),
            payload.len(),
            KEYPAIR.1.as_ptr(),
        )
    };
    assert_eq!(signed, 1, "ED25519_sign failed");
    format!(
        r#"{{"payload":"{}","signature":"{}"}}"#,
        STANDARD.encode(payload),
        hex::encode(signature)
    )
    .into_bytes()
}

fuzz_target!(|data: &[u8]| {
    let Some((mode, rest)) = data.split_first() else {
        return;
    };
    let source = BundleSource::new("fuzz", vec![KEYPAIR.0]).max_lifetime(Duration::MAX);
    let bundle = if mode & 1 == 0 {
        rest.to_vec()
    } else {
        envelope(rest)
    };
    drop(ProfileBundle::from_bytes(&bundle, &source));
});
