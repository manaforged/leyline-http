use std::io::Write as _;

use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub async fn spawn() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handle(sock));
        }
    });
    format!("http://{addr}")
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn read_request(sock: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let req_line = lines.next().unwrap_or("");
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }

    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = sock.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

async fn handle(mut sock: TcpStream) {
    let Some(req) = read_request(&mut sock).await else {
        return;
    };
    let (route, query) = match req.path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (req.path.as_str(), ""),
    };
    let header_val = Value::Object(title_cased_headers(&req.headers));

    let resp = match (req.method.as_str(), route) {
        ("GET", "/gzip") => json_compressed(
            "gzip",
            serde_json::json!({"gzipped": true, "headers": header_val}),
        ),
        ("GET", "/brotli") => json_compressed(
            "br",
            serde_json::json!({"brotli": true, "headers": header_val}),
        ),
        ("GET", "/deflate") => json_compressed(
            "deflate",
            serde_json::json!({"deflated": true, "headers": header_val}),
        ),
        ("GET", "/get") => json_200(serde_json::json!({"url": route, "headers": header_val})),
        ("GET", "/headers") => json_200(serde_json::json!({"headers": header_val})),
        ("GET", "/cookies") => {
            let cookies = req
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                .map(|(_, v)| parse_cookies(v))
                .unwrap_or_default();
            json_200(serde_json::json!({"cookies": Value::Object(cookies)}))
        }
        ("GET", "/cookies/set") => {
            redirect_302("/cookies", vec![("Set-Cookie", format!("{query}; Path=/"))])
        }
        ("GET", p) if p.starts_with("/redirect/") => {
            let n: usize = p.trim_start_matches("/redirect/").parse().unwrap_or(0);
            let loc = if n <= 1 {
                "/get".to_string()
            } else {
                format!("/redirect/{}", n - 1)
            };
            redirect_302(&loc, vec![])
        }
        (_, "/redirect-to") => {
            let params = parse_query(query);
            let target = params.get("url").map(String::as_str).unwrap_or("/get");
            let status: u16 = params
                .get("status_code")
                .and_then(|s| s.parse().ok())
                .unwrap_or(302);
            let reason = match status {
                301 => "Moved Permanently",
                303 => "See Other",
                307 => "Temporary Redirect",
                308 => "Permanent Redirect",
                _ => "Found",
            };
            build(
                status,
                reason,
                vec![("Location", target.to_string())],
                Vec::new(),
            )
        }
        ("POST", "/post") => {
            let ct = req
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let body_str = String::from_utf8_lossy(&req.body).to_string();
            let json_field = if ct.contains("application/json") {
                serde_json::from_slice::<Value>(&req.body).unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let form_field = if ct.contains("x-www-form-urlencoded") {
                parse_form(&body_str)
            } else {
                Map::new()
            };
            json_200(serde_json::json!({
                "json": json_field,
                "form": Value::Object(form_field),
                "data": body_str,
                "headers": header_val,
            }))
        }
        ("GET", "/set-cookie-quoted") => build(
            200,
            "OK",
            vec![
                ("Content-Type", "text/plain".to_string()),
                ("Set-Cookie", "token=\"quoted value\"; Path=/".to_string()),
            ],
            b"ok".to_vec(),
        ),
        _ => json_status(404, "Not Found", serde_json::json!({"error": "not found"})),
    };

    drop(sock.write_all(&resp).await);
    drop(sock.flush().await);
}

fn build(status: u16, reason: &str, extra: Vec<(&str, String)>, body: Vec<u8>) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str("Connection: close\r\n");
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(&body);
    out
}

fn json_200(v: Value) -> Vec<u8> {
    json_status(200, "OK", v)
}

fn json_status(status: u16, reason: &str, v: Value) -> Vec<u8> {
    build(
        status,
        reason,
        vec![("Content-Type", "application/json".to_string())],
        serde_json::to_vec(&v).unwrap(),
    )
}

fn json_compressed(encoding: &str, v: Value) -> Vec<u8> {
    let raw = serde_json::to_vec(&v).unwrap();
    let body = match encoding {
        "gzip" => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&raw).unwrap();
            e.finish().unwrap()
        }
        "deflate" => {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&raw).unwrap();
            e.finish().unwrap()
        }
        "br" => {
            let mut out = Vec::new();
            {
                let mut w = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
                w.write_all(&raw).unwrap();
            }
            out
        }
        other => panic!("unsupported encoding {other}"),
    };
    build(
        200,
        "OK",
        vec![
            ("Content-Type", "application/json".to_string()),
            ("Content-Encoding", encoding.to_string()),
        ],
        body,
    )
}

fn redirect_302(location: &str, mut extra: Vec<(&str, String)>) -> Vec<u8> {
    extra.insert(0, ("Location", location.to_string()));
    build(302, "Found", extra, Vec::new())
}

fn title_cased_headers(headers: &[(String, String)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in headers {
        m.insert(title_case(k), Value::String(v.clone()));
    }
    m
}

fn title_case(name: &str) -> String {
    name.split('-')
        .map(|seg| {
            let mut c = seg.chars();
            match c.next() {
                Some(first) => {
                    first.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn parse_cookies(header: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for pair in header.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            m.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
        }
    }
    m
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            m.insert(k.to_string(), v.to_string());
        }
    }
    m
}

fn parse_form(body: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for pair in body.split('&') {
        if pair.is_empty() {
            continue;
        }
        match pair.split_once('=') {
            Some((k, v)) => {
                m.insert(k.to_string(), Value::String(v.to_string()));
            }
            None => {
                m.insert(pair.to_string(), Value::String(String::new()));
            }
        }
    }
    m
}
