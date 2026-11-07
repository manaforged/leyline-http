use std::env;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use leyline::{Browser, Protocol, Session, SessionBuilder, Timeout, Tls};

/// Configures a session from the CMP_* environment the comparison harness sets.
pub fn session_builder(ca: Option<&[u8]>) -> SessionBuilder {
    let browser = match env::var("LEYLINE_CHROME").unwrap_or_default().as_str() {
        "bare" => None,
        "" => Some(Browser::Chrome152),
        version => Some(
            format!("chrome-{version}")
                .parse::<Browser>()
                .unwrap_or_else(|_| {
                    eprintln!("leyline has no Chrome {version} profile");
                    std::process::exit(1)
                }),
        ),
    };
    let mut b = Session::builder().protocol(if h1() {
        Protocol::Http1
    } else {
        Protocol::Http2
    });
    if env::var_os("CMP_RECONNECT").is_some() {
        b = b.keep_alive(false);
    }
    if let Some(ca) = ca {
        b = b.tls(Tls::new().system_roots(false).env_roots(false).ca_der(ca));
    } else {
        b = b.tls(Tls::new().danger_accept_invalid_certs(true));
    }
    if let Some(timeout) = timeout() {
        b = b.timeout(timeout);
    }
    if let Some(browser) = browser {
        b = b.browser(browser);
    }
    if let Some(n) = env::var("CMP_HOST_CONNS").ok().and_then(|v| v.parse().ok()) {
        b = b.max_connections_per_host(n);
    }
    if let Some(path) = env::var_os("CMP_HEADERS") {
        let headers = fs::read_to_string(path).expect("read request headers");
        b = b.headers(
            headers
                .lines()
                .map(|line| line.split_once('\t').expect("header name and value")),
        );
    }
    if let Ok(p) = std::env::var("PROXY")
        && !p.is_empty()
    {
        b = b.proxy(p);
    }
    b
}

fn timeout() -> Option<Timeout> {
    match env::var("CMP_TIMEOUT").as_deref() {
        Err(_) | Ok("default") => None,
        Ok("none") => Some(Timeout::new().total(None)),
        Ok(v) => Some(
            Timeout::new().total(
                v.strip_suffix('s')
                    .and_then(|n| n.parse().ok())
                    .filter(|&n: &u64| n > 0)
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| {
                        panic!("CMP_TIMEOUT must be default, none or <N>s, got {v}")
                    }),
            ),
        ),
    }
}

/// Returns whether the harness asked for HTTP/1.1.
pub fn h1() -> bool {
    static H1: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| match env::var("CMP_PROTO").as_deref() {
            Ok("h1") => true,
            Ok("h2") | Err(_) => false,
            Ok(other) => panic!("CMP_PROTO must be h1 or h2, got {other}"),
        });
    *H1
}

/// Returns the HTTP version every response must use.
pub fn version() -> leyline::http::Version {
    if h1() {
        leyline::http::Version::HTTP_11
    } else {
        leyline::http::Version::HTTP_2
    }
}

/// Hashes a body for the harness equivalence check.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Returns the body every response must carry.
pub fn expected_body() -> Arc<[u8]> {
    match env::var_os("CMP_BODY") {
        Some(size) => (0..size
            .to_str()
            .expect("CMP_BODY is UTF-8")
            .parse::<usize>()
            .expect("CMP_BODY is a byte count"))
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>()
            .into(),
        None => Arc::from(b"ok-10byte!".as_slice()),
    }
}

/// Time every window-mode task spends before its counted window opens.
pub const WINDOW_WARM: std::time::Duration = std::time::Duration::from_millis(300);

/// Formats a window-mode result from each task's completions inside the window.
pub fn window_line(counts: &[u64], seconds: f64) -> String {
    let total: u64 = counts.iter().sum();
    format!(
        "WINDOW rps={:.1} tasks={} task_min={} task_max={}",
        total as f64 / seconds,
        counts.len(),
        counts.iter().min().copied().unwrap_or(0),
        counts.iter().max().copied().unwrap_or(0)
    )
}
