#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use std::net::SocketAddr;

use leyline::{Browser, Platform, Resolver, Session, Tls};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::tls_support::{self_signed, tls_server};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/wire_golden.txt"
);
const PREFACE_LEN: usize = 24;
const HEADERS: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const SERVER_SETTINGS: [u8; 18] = [0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 4, 1, 0, 0, 0, 0];
const ORIGIN: &str = "origin.test";
const WAIT: Duration = Duration::from_secs(3);

#[tokio::test(flavor = "multi_thread")]
async fn every_profile_sends_its_recorded_client_hello_and_h2_opening() {
    let cases = Browser::all().iter().flat_map(|&browser| {
        Platform::all().iter().filter_map(move |&platform| {
            Session::builder()
                .browser(browser)
                .platform(platform)
                .build()
                .ok()
                .map(|_| (browser, platform))
        })
    });
    let rendered: String = futures_util::future::join_all(cases.map(|(browser, platform)| {
        tokio::spawn(async move {
            let hello = client_hello(browser, platform).await;
            let hello: String = if browser.profile().tls.permute_extensions {
                hello
                    .lines()
                    .filter(|line| !line.starts_with("tls extension_order "))
                    .map(|line| format!("{line}\n"))
                    .collect()
            } else {
                hello
            };
            let opening = h2_opening(browser, platform).await;
            format!("== {} {platform:?}\n{hello}{opening}", browser.id())
        })
    }))
    .await
    .into_iter()
    .map(Result::unwrap)
    .collect();
    if std::env::var_os("LEYLINE_BLESS").is_some() {
        std::fs::write(GOLDEN, &rendered).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    let drift: Vec<_> = golden
        .split("== ")
        .zip(rendered.split("== "))
        .filter(|(want, got)| want != got)
        .map(|(want, got)| format!("--- golden\n{want}+++ sent\n{got}"))
        .collect();
    assert!(drift.is_empty(), "wire drift:\n{}", drift.join("\n"));
    assert_eq!(golden, rendered, "profile set changed");
}

async fn client_hello(browser: Browser, platform: Platform) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_record(&mut stream).await
    });
    let session = Session::builder()
        .browser(browser)
        .platform(platform)
        .timeout(WAIT)
        .build()
        .unwrap();
    drop(
        session
            .get(format!("https://localhost:{port}/"))
            .send()
            .await,
    );
    describe_client_hello(&server.await.unwrap())
}

async fn h2_opening(browser: Browser, platform: Platform) -> String {
    let (cert, key) = self_signed();
    let (sent, mut frames) = mpsc::unbounded_channel();
    let port = tls_server(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        move |mut stream| {
            let sent = sent.clone();
            async move {
                let mut preface = [0u8; PREFACE_LEN];
                if stream.read_exact(&mut preface).await.is_err() {
                    return;
                }
                drop(sent.send(String::from_utf8_lossy(&preface).escape_debug().to_string()));
                if stream.write_all(&SERVER_SETTINGS).await.is_err() {
                    return;
                }
                while let Some((kind, flags, stream_id, payload)) = read_frame(&mut stream).await {
                    let line = format!("frame {kind} {flags:#04x} {stream_id} {}", hex(&payload));
                    drop(sent.send(line));
                    if kind == HEADERS && flags & END_HEADERS != 0 {
                        break;
                    }
                }
            }
        },
    )
    .await;
    let session = Session::builder()
        .browser(browser)
        .platform(platform)
        .dns_resolver(Arc::new(Loopback(port)))
        .tls(Tls::new().danger_accept_invalid_certs(true))
        .timeout(WAIT)
        .build()
        .unwrap();
    let request = tokio::spawn(async move {
        drop(session.get(format!("https://{ORIGIN}/golden")).send().await);
    });
    let mut out = String::new();
    while let Ok(Some(line)) = tokio::time::timeout(WAIT, frames.recv()).await {
        let done = line.starts_with(&format!("frame {HEADERS} "));
        writeln!(out, "h2 {line}").unwrap();
        if done {
            break;
        }
    }
    request.abort();
    out
}

struct Loopback(u16);

impl Resolver for Loopback {
    fn resolve<'a>(
        &'a self,
        _host: &'a str,
        _port: u16,
    ) -> std::pin::Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send + 'a>> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.0));
        Box::pin(async move { Ok(vec![addr]) })
    }
}

async fn read_frame<S: AsyncRead + Unpin>(s: &mut S) -> Option<(u8, u8, u32, Vec<u8>)> {
    let mut head = [0u8; 9];
    tokio::time::timeout(WAIT, s.read_exact(&mut head))
        .await
        .ok()?
        .ok()?;
    let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
    let stream_id = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) & 0x7fff_ffff;
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).await.ok()?;
    Some((head[3], head[4], stream_id, payload))
}

async fn read_record(stream: &mut TcpStream) -> Vec<u8> {
    let mut record = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(Ok(n)) = tokio::time::timeout(WAIT, stream.read(&mut buf)).await {
        if n == 0 {
            break;
        }
        record.extend_from_slice(&buf[..n]);
        if record.len() >= 5 && record.len() >= 5 + usize::from(u16_at(&record, 3)) {
            break;
        }
    }
    record
}

fn describe_client_hello(record: &[u8]) -> String {
    let mut out = String::new();
    let hello = &record[9..];
    writeln!(
        out,
        "tls record {:04x} hello {:04x}",
        u16_at(record, 1),
        u16_at(hello, 0)
    )
    .unwrap();
    let mut at = 34;
    writeln!(out, "tls session_id_len {}", hello[at]).unwrap();
    at += 1 + usize::from(hello[at]);
    let ciphers_len = usize::from(u16_at(hello, at));
    let ciphers: Vec<_> = hello[at + 2..at + 2 + ciphers_len]
        .chunks(2)
        .map(|c| grease_or_hex(u16::from_be_bytes([c[0], c[1]])))
        .collect();
    writeln!(out, "tls ciphers {}", ciphers.join(",")).unwrap();
    at += 2 + ciphers_len;
    writeln!(
        out,
        "tls compression {}",
        hex(&hello[at + 1..at + 1 + usize::from(hello[at])])
    )
    .unwrap();
    at += 1 + usize::from(hello[at]);
    let end = at + 2 + usize::from(u16_at(hello, at));
    at += 2;
    let mut extensions = Vec::new();
    while at + 4 <= end {
        let kind = u16_at(hello, at);
        let len = usize::from(u16_at(hello, at + 2));
        let body = &hello[at + 4..at + 4 + len];
        extensions.push((grease_or_hex(kind), describe_extension(kind, body)));
        at += 4 + len;
    }
    let order: Vec<_> = extensions.iter().map(|(kind, _)| kind.clone()).collect();
    writeln!(out, "tls extension_order {}", order.join(",")).unwrap();
    extensions.sort();
    for (kind, body) in extensions {
        writeln!(out, "tls ext {kind} {body}").unwrap();
    }
    out
}

fn describe_extension(kind: u16, body: &[u8]) -> String {
    match kind {
        0x0033 => {
            let mut at = 2;
            let mut shares = Vec::new();
            while at + 4 <= body.len() {
                let len = usize::from(u16_at(body, at + 2));
                shares.push(format!("{}:{len}", grease_or_hex(u16_at(body, at))));
                at += 4 + len;
            }
            format!("key_shares {}", shares.join(","))
        }
        0xfe0d => "ech_grease".to_owned(),
        0x0029 => format!("psk len {}", body.len()),
        0x002b => grease_body(body, 1),
        _ => grease_body(body, 0),
    }
}

fn grease_body(body: &[u8], align: usize) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < body.len() {
        if at >= align
            && (at - align).is_multiple_of(2)
            && at + 1 < body.len()
            && is_grease(u16_at(body, at))
        {
            out.push_str("GGGG");
            at += 2;
        } else {
            write!(out, "{:02x}", body[at]).unwrap();
            at += 1;
        }
    }
    out
}

fn grease_or_hex(value: u16) -> String {
    if is_grease(value) {
        "GREASE".to_owned()
    } else {
        format!("{value:04x}")
    }
}

fn is_grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a && value >> 8 == value & 0xff
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").unwrap();
        out
    })
}
