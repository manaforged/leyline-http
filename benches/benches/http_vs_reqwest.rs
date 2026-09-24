use std::time::{Duration, Instant};

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use futures_util::future::join_all;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

use leyline::Session;

const RESP: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: keep-alive\r\n\r\nok-10byte!";

async fn handle_conn(mut sock: TcpStream) {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 8192];
    loop {
        while let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.drain(..pos + 4);
            if sock.write_all(RESP).await.is_err() {
                return;
            }
        }
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
}

async fn run_server(listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((sock, _)) => {
                let _ = sock.set_nodelay(true);
                tokio::spawn(handle_conn(sock));
            }
            Err(_) => return,
        }
    }
}

async fn spawn_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(run_server(listener));
    format!("http://{addr}/")
}

const SERIAL_N: usize = 1000;
const CONC_N: usize = 100;

fn bench_leyline_serial(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let url = rt.block_on(spawn_server());
    let session = Session::builder().build().expect("leyline session");
    rt.block_on(async {
        session.get(&url).await.expect("warm");
    });

    let mut g = c.benchmark_group("http_vs_reqwest");
    g.throughput(Throughput::Elements(SERIAL_N as u64));
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("leyline_serial_1k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..SERIAL_N {
                        let resp = session.get(&url).await.expect("req ok");
                        black_box(resp);
                    }
                    total += start.elapsed();
                }
                total
            })
        })
    });
    g.finish();
}

fn bench_reqwest_serial(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let url = rt.block_on(spawn_server());
    let client = reqwest::Client::new();
    rt.block_on(async {
        client
            .get(&url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    });

    let mut g = c.benchmark_group("http_vs_reqwest");
    g.throughput(Throughput::Elements(SERIAL_N as u64));
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("reqwest_serial_1k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..SERIAL_N {
                        let body = client
                            .get(&url)
                            .send()
                            .await
                            .expect("req ok")
                            .bytes()
                            .await
                            .expect("body");
                        black_box(body);
                    }
                    total += start.elapsed();
                }
                total
            })
        })
    });
    g.finish();
}

fn bench_leyline_concurrent(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let url = rt.block_on(spawn_server());
    let session = Session::builder().build().expect("leyline session");
    rt.block_on(async {
        session.get(&url).await.expect("warm");
    });

    let mut g = c.benchmark_group("http_vs_reqwest");
    g.throughput(Throughput::Elements(CONC_N as u64));
    g.sample_size(30);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("leyline_concurrent_100", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut futs = Vec::with_capacity(CONC_N);
                    for _ in 0..CONC_N {
                        let s = session.clone();
                        let u = url.clone();
                        futs.push(tokio::spawn(
                            async move { s.get(&u).await.expect("req ok") },
                        ));
                    }
                    let results = join_all(futs).await;
                    total += start.elapsed();
                    for r in results {
                        black_box(r.unwrap());
                    }
                }
                total
            })
        })
    });
    g.finish();
}

fn bench_reqwest_concurrent(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let url = rt.block_on(spawn_server());
    let client = reqwest::Client::new();
    rt.block_on(async {
        client
            .get(&url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    });

    let mut g = c.benchmark_group("http_vs_reqwest");
    g.throughput(Throughput::Elements(CONC_N as u64));
    g.sample_size(30);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("reqwest_concurrent_100", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut futs = Vec::with_capacity(CONC_N);
                    for _ in 0..CONC_N {
                        let client = client.clone();
                        let u = url.clone();
                        futs.push(tokio::spawn(async move {
                            client
                                .get(&u)
                                .send()
                                .await
                                .expect("req ok")
                                .bytes()
                                .await
                                .expect("body")
                        }));
                    }
                    let results = join_all(futs).await;
                    total += start.elapsed();
                    for r in results {
                        black_box(r.unwrap());
                    }
                }
                total
            })
        })
    });
    g.finish();
}

criterion_group!(
    http_vs_reqwest,
    bench_leyline_serial,
    bench_reqwest_serial,
    bench_leyline_concurrent,
    bench_reqwest_concurrent
);
criterion_main!(http_vs_reqwest);
