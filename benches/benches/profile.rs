
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use leyline::Browser;
use leyline::profile::ALL_BROWSERS;

fn bench_profile_lookup_chrome147(c: &mut Criterion) {
    c.bench_function("profile::lookup_chrome147", |b| {
        b.iter(|| {
            let p = black_box(Browser::Chrome147).profile();
            black_box(p);
        });
    });
}

fn bench_profile_lookup_all(c: &mut Criterion) {
    c.bench_function("profile::lookup_all_browsers", |b| {
        b.iter(|| {
            for br in ALL_BROWSERS {
                black_box(br.profile());
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
