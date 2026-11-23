use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::cookie::{Cookie, Jar};
use leyline::testing::{Response, Server};
use leyline::{Browser, Proxy, Session};

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn range(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }

    #[cfg(feature = "bench-internals")]
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push((self.next_u64() & 0xff) as u8);
        }
        out
    }
}

#[cfg(feature = "bench-internals")]
#[test]
fn prop_h1_head_ows_and_framing_invariants() {
    use leyline::internals::fuzz::parse_h1_head;

    let mut rng = SplitMix64::new(0x5eed_cafe_1234_5678);
    let ows_samples: &[&[u8]] = &[b"", b" ", b"\t", b"  \t ", b"\t  \t"];
    let values: &[&[u8]] = &[b"ok", b"a b", b"v=1; x=2", b"hello\tworld", b"12345"];

    for case in 0..400 {
        let status = 100 + (rng.range(500) as u16);
        let minor = rng.range(2);
        let mut head = format!("HTTP/1.{minor} {status} OK\r\n").into_bytes();
        let count = 1 + rng.range(6);
        for i in 0..count {
            let lead = ows_samples[rng.range(ows_samples.len())];
            let trail = ows_samples[rng.range(ows_samples.len())];
            let val = values[rng.range(values.len())];
            head.extend_from_slice(format!("X-H{i}:").as_bytes());
            head.extend_from_slice(lead);
            head.extend_from_slice(val);
            head.extend_from_slice(trail);
            head.extend_from_slice(b"\r\n");
        }
        head.extend_from_slice(b"\r\n");

        let (parsed_status, headers, parsed_minor) =
            parse_h1_head(&head).unwrap_or_else(|e| panic!("case {case}: {e:?}"));
        assert_eq!(parsed_status, status);
        assert_eq!(parsed_minor, minor as u8);
        for (_, val) in &headers {
            let b = val.as_bytes();
            assert!(
                !matches!(b.first(), Some(b' ' | b'\t')),
                "case {case}: leading OWS leaked: {b:?}"
            );
            assert!(
                !matches!(b.last(), Some(b' ' | b'\t')),
                "case {case}: trailing OWS leaked: {b:?}"
            );
        }
    }

    for _ in 0..500 {
        let len = rng.range(128);
        let noise = rng.bytes(len);
        if let Ok((status, headers, minor)) = parse_h1_head(&noise) {
            assert!((100..=999).contains(&status));
            assert!(minor <= 1);
            assert!(
                !(headers.contains_key("content-length")
                    && headers.contains_key("transfer-encoding"))
            );
            assert!(headers.get_all("content-length").iter().count() <= 1);
            for (_, val) in &headers {
                let b = val.as_bytes();
                assert!(!matches!(b.first(), Some(b' ' | b'\t')));
                assert!(!matches!(b.last(), Some(b' ' | b'\t')));
            }
        }
    }
}

#[cfg(feature = "bench-internals")]
#[tokio::test]
async fn prop_h1_chunked_random_split_invariance() {
    use leyline::internals::fuzz::read_chunked_body;
    use tokio::io::AsyncWriteExt;

    let mut rng = SplitMix64::new(0x0c4a_a7ed_9876_5432);

    for case in 0..80 {
        let n_chunks = 1 + rng.range(6);
        let mut expected = Vec::new();
        let mut wire = Vec::new();
        for _ in 0..n_chunks {
            let clen = 1 + rng.range(40);
            let data = rng.bytes(clen);
            expected.extend_from_slice(&data);
            wire.extend_from_slice(format!("{clen:x}\r\n").as_bytes());
            wire.extend_from_slice(&data);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0\r\n\r\n");

        let split = rng.range(wire.len() + 1);
        let (mut reader, mut writer) = tokio::io::duplex(wire.len() + 64);
        let wire_copy = wire.clone();
        let seed_split = 1 + rng.range(7);
        let writer_task = tokio::spawn(async move {
            let mut pos = split;
            while pos < wire_copy.len() {
                let end = (pos + seed_split).min(wire_copy.len());
                writer.write_all(&wire_copy[pos..end]).await.unwrap();
                pos = end;
            }
            drop(writer.shutdown().await);
        });
        let initial = wire[..split].to_vec();
        let body = read_chunked_body(&mut reader, initial, 65_536)
            .await
            .unwrap_or_else(|e| panic!("case {case}: {e:?}"));
        writer_task.await.unwrap();
        assert_eq!(body, expected, "case {case} mismatch");
    }
}

#[cfg(feature = "bench-internals")]
#[test]
fn prop_dns_wire_never_panics_on_adversarial_packets() {
    use leyline::internals::fuzz::parse_dns_response;

    let mut rng = SplitMix64::new(0x0d15_ea5e_1020_3040);
    for _ in 0..500 {
        let len = rng.range(256);
        let mut pkt = rng.bytes(len);
        if pkt.len() >= 12 {
            pkt[2] |= 0x80;
            pkt[3] &= 0xf0;
        }
        parse_dns_response(&pkt);
    }
    let loop_pkt = [
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x0c, 0x00,
        0x01, 0x00, 0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04,
        127, 0, 0, 1,
    ];
    parse_dns_response(&loop_pkt);
}

#[test]
fn prop_cookie_jar_security_prefixes_and_persistence_roundtrip() {
    let mut rng = SplitMix64::new(0x0c00_c1e5_abcd_ef01);
    let dir = super::v030_workloads::TempDir::new("prop-cookie");
    let jar = Jar::new();
    let https_root = "https://app.example.com/a/b";
    let http_root = "http://app.example.com/a/b";

    for step in 0..150 {
        let name = format!("k{}", rng.range(12));
        let val = format!("v{step}");
        match rng.range(5) {
            0 => {
                jar.store(http_root, &format!("__Secure-{name}={val}; Secure; Path=/"));
                assert!(
                    !jar.cookies_for(https_root)
                        .iter()
                        .any(|c| c.name() == format!("__Secure-{name}") && c.value() == val),
                    "__Secure- accepted from http origin"
                );
            }
            1 => {
                jar.store(
                    https_root,
                    &format!("__Host-{name}={val}; Secure; Domain=example.com; Path=/"),
                );
                assert!(
                    !jar.cookies_for(https_root)
                        .iter()
                        .any(|c| c.name() == format!("__Host-{name}") && c.value() == val),
                    "__Host- accepted with Domain attribute"
                );
            }
            2 => {
                jar.store(https_root, &format!("__Host-{name}={val}; Secure; Path=/"));
                assert!(
                    jar.cookies_for(https_root)
                        .iter()
                        .any(|c| c.name() == format!("__Host-{name}") && c.value() == val)
                );
                assert!(
                    !jar.cookies_for(http_root)
                        .iter()
                        .any(|c| c.name() == format!("__Host-{name}")),
                    "Secure cookie leaked to http"
                );
            }
            3 => {
                let c = Cookie::builder(name.clone(), val.clone())
                    .path("/a")
                    .secure(false)
                    .build();
                jar.insert(https_root, c);
                assert!(
                    jar.cookies_for(https_root)
                        .iter()
                        .any(|c| c.name() == name && c.value() == val)
                );
            }
            _ => {
                jar.store(https_root, &format!("{name}=gone; Max-Age=0; Path=/a"));
            }
        }
    }

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .cookies(jar.clone())
        .build()
        .unwrap();
    let save_path = dir.path("state.json");
    session.save(&save_path).unwrap();
    let loaded = Session::load(&save_path).unwrap().build().unwrap();

    let mut before: Vec<(String, String)> = session
        .cookies()
        .cookies_for(https_root)
        .into_iter()
        .map(|c| (c.name().to_owned(), c.value().to_owned()))
        .collect();
    let loaded_cookies = loaded.cookies().cookies_for(https_root);
    for pair in loaded_cookies.windows(2) {
        assert!(
            pair[0].path().len() >= pair[1].path().len(),
            "RFC 6265 path specificity violated after load"
        );
    }
    let mut after: Vec<(String, String)> = loaded_cookies
        .into_iter()
        .map(|c| (c.name().to_owned(), c.value().to_owned()))
        .collect();
    before.sort();
    after.sort();
    assert_eq!(before, after);
}

#[tokio::test]
async fn prop_proxy_pool_state_machine_invariants() {
    let origin = Server::http(|req| {
        if req.target().starts_with("/fail") {
            Response::new(503).body("busy")
        } else {
            Response::new(200).body("direct")
        }
    })
    .await
    .unwrap();
    let origin_port = origin.addr().port();

    let proxy_a_hits = Arc::new(AtomicUsize::new(0));
    let pa = Arc::clone(&proxy_a_hits);
    let proxy_a = Server::http(move |req| {
        pa.fetch_add(1, Ordering::SeqCst);
        if req.target().contains("/boom") {
            Response::new(403)
                .header("cf-mitigated", "challenge")
                .body("blocked")
        } else {
            Response::new(200).body("via-a")
        }
    })
    .await
    .unwrap();

    let proxy_b_hits = Arc::new(AtomicUsize::new(0));
    let pb = Arc::clone(&proxy_b_hits);
    let proxy_b = Server::http(move |_| {
        pb.fetch_add(1, Ordering::SeqCst);
        Response::new(200).body("via-b")
    })
    .await
    .unwrap();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(
            Proxy::list([
                Proxy::from(format!("http://{}", proxy_a.addr()))
                    .no_proxy([format!("127.0.0.1:{origin_port}")]),
                Proxy::from(format!("http://{}", proxy_b.addr()))
                    .no_proxy([format!("127.0.0.1:{origin_port}")]),
            ])
            .ban_after(2)
            .ban_for(Duration::from_millis(60)),
        )
        .build()
        .unwrap();

    let mut rng = SplitMix64::new(0x0b00_1e55_4455_6677);
    for step in 0..24 {
        if step % 3 == 0 {
            let text = session.get(origin.url("/direct")).text().await.unwrap();
            assert_eq!(text, "direct");
        } else {
            let path = if rng.range(3) == 0 { "/boom" } else { "/ok" };
            let resp = session
                .get(format!("http://127.0.0.1:1{path}"))
                .send()
                .await
                .unwrap();
            assert!(matches!(resp.status().as_u16(), 200 | 403));
        }
        let stats = session.stats();
        for p in stats.proxies() {
            assert_eq!(stats.proxy_in_use(p), 0, "in_use leaked after step {step}");
        }
    }
    assert!(proxy_a_hits.load(Ordering::SeqCst) > 0);
    assert!(proxy_b_hits.load(Ordering::SeqCst) > 0);
}
