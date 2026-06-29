//! Static profile lookup cost — `leyline::profile(Browser::Chrome147)`.
//!
//! The registry is built once behind a `LazyLock`, so all iterations after
//! the first are pure HashMap dispatch. The first iteration pays the
//! registry-build cost — criterion's warm-up handles that automatically.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use leyline::{Browser, profile};

fn bench_profile_lookup_chrome147(c: &mut Criterion) {
    c.bench_function("profile::lookup_chrome147", |b| {
        b.iter(|| {
            let p = profile(black_box(Browser::Chrome147));
            black_box(p);
        });
    });
}

fn bench_profile_lookup_all(c: &mut Criterion) {
    // Walk every browser variant — amortises HashMap dispatch across the
    // full registry to catch pathological lookups.
    c.bench_function("profile::lookup_all_browsers", |b| {
        b.iter(|| {
            for br in leyline::ALL_BROWSERS {
                black_box(profile(br));
            }
        });
    });
}

criterion_group!(
    profile_benches,
    bench_profile_lookup_chrome147,
    bench_profile_lookup_all
);
criterion_main!(profile_benches);
