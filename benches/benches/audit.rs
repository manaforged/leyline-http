//! JA3 / JA4 / JA4T computation cost.
//!
//! We feed each function the same profile-derived input on every iteration
//! so we're measuring the fingerprint math (hashing + sorting + formatting)
//! rather than profile lookup. Chrome 147 is the reference shape.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use leyline_audit::{
    chrome_extension_ids, compute_ja3, compute_ja4, compute_ja4t, Ja3Input, Ja4Input,
};
use leyline_profile::{Browser, ProfileRegistry};

fn chrome_147_inputs() -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<u16>,
) {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles
        .get_browser(Browser::Chrome147)
        .expect("Chrome147 profile must exist in the builtin registry");
    let ciphers = profile.tls.ciphers.clone();
    let sigalgs = profile.tls.sigalgs.clone();
    let curves = profile.tls.curves.clone();
    let ext_ids = chrome_extension_ids(&profile.tls);
    (ciphers, sigalgs, curves, ext_ids)
}

fn bench_ja3(c: &mut Criterion) {
    let (ciphers, _sigalgs, curves, ext_ids) = chrome_147_inputs();
    c.bench_function("audit::compute_ja3", |b| {
        b.iter(|| {
            let input = Ja3Input {
                ciphers: black_box(&ciphers),
                curves: black_box(&curves),
                extension_ids: black_box(&ext_ids),
                tls_record_version: 771,
            };
            black_box(compute_ja3(&input));
        });
    });
}

fn bench_ja4(c: &mut Criterion) {
    let (ciphers, sigalgs, curves, ext_ids) = chrome_147_inputs();
    c.bench_function("audit::compute_ja4", |b| {
        b.iter(|| {
            let input = Ja4Input {
                ciphers: black_box(&ciphers),
                sigalgs: black_box(&sigalgs),
                curves: black_box(&curves),
                extension_ids: black_box(&ext_ids),
                tls_version: "1.3",
                has_sni: true,
                alpn: "h2",
            };
            black_box(compute_ja4(&input));
        });
    });
}

fn bench_ja4t(c: &mut Criterion) {
    c.bench_function("audit::compute_ja4t", |b| {
        b.iter(|| {
            black_box(compute_ja4t(
                black_box(65535),
                black_box(1460),
                black_box(8),
                black_box(true),
            ));
        });
    });
}

fn bench_extension_ids(c: &mut Criterion) {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles.get_browser(Browser::Chrome147).unwrap();
    c.bench_function("audit::chrome_extension_ids", |b| {
        b.iter(|| {
            black_box(chrome_extension_ids(black_box(&profile.tls)));
        });
    });
}

criterion_group!(
    audit_benches,
    bench_ja3,
    bench_ja4,
    bench_ja4t,
    bench_extension_ids
);
criterion_main!(audit_benches);
