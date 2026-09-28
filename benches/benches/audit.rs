
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use leyline::audit::{
    Ja3Input, Ja4Input, Ja4hInput, compute_ja3, compute_ja4, compute_ja4h, compute_ja4t,
};
use leyline::fuzz::extension_ids;
use leyline::profile::ProfileRegistry;
use leyline::{Browser, Platform};

fn chrome_147_inputs() -> (Vec<String>, Vec<String>, Vec<String>, Vec<u16>) {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles
        .get_browser(Browser::Chrome147)
        .expect("Chrome147 profile must exist in the builtin registry");
    let ciphers = profile.tls.ciphers.clone();
    let sigalgs = profile.tls.sigalgs.clone();
    let curves = profile.tls.curves.clone();
    let ext_ids = extension_ids(&profile.tls);
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
    let tcp = Platform::Windows.tcp_profile();
    c.bench_function("audit::compute_ja4t", |b| {
        b.iter(|| {
            black_box(compute_ja4t(black_box(&tcp)));
        });
    });
}

fn bench_extension_ids(c: &mut Criterion) {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles.get_browser(Browser::Chrome147).unwrap();
    c.bench_function("audit::extension_ids", |b| {
        b.iter(|| {
            black_box(extension_ids(black_box(&profile.tls)));
        });
    });
}

fn bench_ja4h(c: &mut Criterion) {
    let headers: Vec<(String, String)> = [
        (":method", "GET"),
        (":authority", "example.com"),
        (":scheme", "https"),
        (":path", "/"),
        ("sec-ch-ua", "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\""),
        ("sec-ch-ua-mobile", "?0"),
        ("sec-ch-ua-platform", "\"Windows\""),
        ("upgrade-insecure-requests", "1"),
        ("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36"),
        ("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"),
        ("sec-fetch-site", "none"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-user", "?1"),
        ("sec-fetch-dest", "document"),
        ("accept-encoding", "gzip, deflate, br, zstd"),
        ("accept-language", "en-US,en;q=0.9"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    c.bench_function("audit::compute_ja4h", |b| {
        b.iter(|| {
            let input = Ja4hInput {
                method: black_box("GET"),
                http_version: black_box("2"),
                headers: black_box(&headers),
            };
            black_box(compute_ja4h(&input));
        });
    });
}

criterion_group!(
    audit_benches,
    bench_ja3,
    bench_ja4,
    bench_ja4t,
    bench_extension_ids,
    bench_ja4h
);
criterion_main!(audit_benches);
