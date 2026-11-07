use criterion::{Criterion, black_box, criterion_group, criterion_main};
use leyline::Browser;
use leyline::internals::browser_profile;

fn bench_profile_lookup_chrome147(c: &mut Criterion) {
    c.bench_function("profile::lookup_chrome147", |b| {
        b.iter(|| {
            let p = browser_profile(black_box(Browser::Chrome147));
            black_box(p);
        });
    });
}

fn bench_profile_lookup_all(c: &mut Criterion) {
    c.bench_function("profile::lookup_all_browsers", |b| {
        b.iter(|| {
            for br in Browser::all().iter().copied() {
                black_box(browser_profile(br));
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
