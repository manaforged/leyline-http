use std::env;
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use leyline::pool::floor::{Floor, floor_connect, floor_connect_async};

fn request_headers() -> Vec<(String, String)> {
    env::var_os("CMP_HEADERS")
        .map(|path| {
            fs::read_to_string(path)
                .expect("read request headers")
                .lines()
                .map(|line| {
                    let (name, value) = line.split_once('\t').expect("header name and value");
                    (name.to_owned(), value.to_owned())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[tokio::main]
async fn warm_async(
    addr: String,
    url: url::Url,
    headers: Arc<Vec<(String, String)>>,
    warm_n: usize,
) -> f64 {
    tokio::spawn(async move {
        let expected = b"ok-10byte!";
        let mut stream = floor_connect_async(&addr).await.expect("connect");
        let mut floor = Floor::new(&headers);
        let start = Instant::now();
        for _ in 0..warm_n {
            let (status, body) = floor
                .get_async(&mut stream, &url)
                .await
                .expect("warm request");
            assert!(status == 200 && body[..] == expected[..]);
        }
        warm_n as f64 / start.elapsed().as_secs_f64()
    })
    .await
    .expect("client task")
}

fn main() {
    if env::var("CMP_PROTO").as_deref() != Ok("h1") {
        eprintln!("the floor client speaks only HTTP/1.1; set CMP_PROTO=h1");
        std::process::exit(1);
    }
    let args: Vec<String> = env::args().collect();
    let url = url::Url::parse(&args[1]).expect("url");
    let addr = format!(
        "{}:{}",
        url.host_str().expect("host"),
        url.port().expect("port")
    );
    let headers = Arc::new(request_headers());
    let expected = b"ok-10byte!";

    if args.get(2).map(String::as_str) == Some("equiv") {
        let mut stream = floor_connect(&addr).expect("connect");
        let (status, body) = Floor::new(&headers)
            .get(&mut stream, &url)
            .expect("request");
        println!(
            "EQUIV floor status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args
        .get(5)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
        .max(1);

    if env::var("CMP_FLOOR").as_deref() == Ok("async") {
        let warm_rps = warm_async(addr, url, headers, warm_n.max(1));
        println!("RESULT floor connections={conc_c} warm_rps={warm_rps:.3} conc_rps=0 cold_rps=0");
        return;
    }

    let mut stream = floor_connect(&addr).expect("connect");
    let mut floor = Floor::new(&headers);
    let start = Instant::now();
    for _ in 0..warm_n.max(1) {
        let (status, body) = floor.get(&mut stream, &url).expect("warm request");
        assert!(status == 200 && body[..] == expected[..]);
    }
    let warm_rps = warm_n.max(1) as f64 / start.elapsed().as_secs_f64();
    drop(stream);

    let per = conc_n / conc_c;
    let ready = Arc::new(Barrier::new(conc_c + 1));
    let handles: Vec<_> = (0..conc_c)
        .map(|_| {
            let (addr, url, headers, ready) = (
                addr.clone(),
                url.clone(),
                Arc::clone(&headers),
                Arc::clone(&ready),
            );
            thread::spawn(move || {
                let mut stream = floor_connect(&addr).expect("connect");
                let mut floor = Floor::new(&headers);
                floor.get(&mut stream, &url).expect("warm request");
                ready.wait();
                for _ in 0..per {
                    let (status, body) = floor.get(&mut stream, &url).expect("request");
                    assert!(status == 200 && body[..] == expected[..]);
                }
            })
        })
        .collect();
    ready.wait();
    let start = Instant::now();
    for handle in handles {
        handle.join().expect("join");
    }
    let conc_rps = (per * conc_c) as f64 / start.elapsed().as_secs_f64();
    println!(
        "RESULT floor connections={conc_c} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps=0"
    );
}
