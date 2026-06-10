//! Session construction cost — `Session::builder().browser(...).build()?`.
//!
//! This covers TLS connector wiring, H2 config derivation, and precomputed
//! JA3/JA4/JA4T/H2-fingerprint strings. Network activity is never touched.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use leyline::{Browser, Session};

fn bench_session_build_chrome147(c: &mut Criterion) {
    c.bench_function("session::build_chrome147", |b| {
        b.iter(|| {
            let session = Session::builder()
                .browser(black_box(Browser::Chrome147))
                .build()
                .expect("Chrome147 session should build");
            black_box(session);
        });
    });
}

fn bench_session_build_chrome_latest(c: &mut Criterion) {
    c.bench_function("session::chrome_latest", |b| {
        b.iter(|| {
            let session = Session::chrome_latest().expect("chrome_latest should build");
            black_box(session);
        });
    });
}

criterion_group!(
    session_benches,
    bench_session_build_chrome147,
    bench_session_build_chrome_latest
);
criterion_main!(session_benches);
