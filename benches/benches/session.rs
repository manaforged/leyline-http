use criterion::{Criterion, black_box, criterion_group, criterion_main};
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

fn bench_session_build_chrome(c: &mut Criterion) {
    c.bench_function("session::chrome", |b| {
        b.iter(|| {
            let session = Session::new();
            black_box(session);
        });
    });
}

criterion_group!(
    session_benches,
    bench_session_build_chrome147,
    bench_session_build_chrome
);
criterion_main!(session_benches);
