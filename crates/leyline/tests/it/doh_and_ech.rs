use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::testing::{Response, Server};
use leyline::{Kind, Session, Tls};
use leyline_bssl::ssl::{NameType, Ssl, SslContextBuilder, SslMethod, SslVersion};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::tls_support::{build_ech_config_list, self_signed_dns_names, serve_h1_ok};

fn decode_qname_and_qtype(query: &[u8]) -> Option<(String, u16)> {
    if query.len() < 17 {
        return None;
    }
    let mut pos = 12;
    let mut labels = Vec::new();
    loop {
        let len = usize::from(*query.get(pos)?);
        pos += 1;
        if len == 0 {
            break;
        }
        let label = std::str::from_utf8(query.get(pos..pos + len)?).ok()?;
        labels.push(label.to_ascii_lowercase());
        pos += len;
    }
    let qtype = u16::from_be_bytes([*query.get(pos)?, *query.get(pos + 1)?]);
    Some((labels.join("."), qtype))
}

fn dns_response(query: &[u8], rcode: u8, answers: &[(u16, u32, &[u8])]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&query[..2]);
    let flags: u16 = 0x8180 | u16::from(rcode & 0x0F);
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&query[12..]);
    for &(rtype, ttl, rdata) in answers {
        out.extend_from_slice(&[0xC0, 0x0C]);
        out.extend_from_slice(&rtype.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&ttl.to_be_bytes());
        out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(rdata);
    }
    out
}

fn svcb_ech_rdata(priority: u16, ech_list: &[u8]) -> Vec<u8> {
    let mut rdata = Vec::with_capacity(ech_list.len() + 8);
    rdata.extend_from_slice(&priority.to_be_bytes());
    rdata.push(0);
    rdata.extend_from_slice(&5u16.to_be_bytes());
    rdata.extend_from_slice(&(ech_list.len() as u16).to_be_bytes());
    rdata.extend_from_slice(ech_list);
    rdata
}

#[test]
fn doh_rejects_plain_http_endpoint() {
    let err = Session::builder()
        .doh("http://127.0.0.1:8080/dns-query")
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
}

#[tokio::test]
async fn doh_resolves_a_records_caches_ttl_and_maps_nxdomain_to_dns_error() {
    let origin = Server::http(|_| Response::new(200).body("via-doh"))
        .await
        .unwrap();
    let origin_port = origin.addr().port();

    let queries = Arc::new(AtomicUsize::new(0));
    let queries_bg = Arc::clone(&queries);
    let doh_server = Server::https(move |req| {
        assert_eq!(req.header("content-type"), Some("application/dns-message"));
        queries_bg.fetch_add(1, Ordering::SeqCst);
        let Some((name, qtype)) = decode_qname_and_qtype(req.body()) else {
            return Response::new(400);
        };
        let wire = match (name.as_str(), qtype) {
            ("app.doh.test", 1) => dns_response(req.body(), 0, &[(1, 300, &[127, 0, 0, 1])]),
            ("app.doh.test", 28) => dns_response(req.body(), 0, &[]),
            ("missing.doh.test", _) => dns_response(req.body(), 3, &[]),
            ("servfail.doh.test", _) => dns_response(req.body(), 2, &[]),
            ("corrupt.doh.test", _) => b"not-a-dns-packet".to_vec(),
            _ => dns_response(req.body(), 3, &[]),
        };
        Response::new(200)
            .header("content-type", "application/dns-message")
            .body(wire)
    })
    .await
    .unwrap();

    let session = Session::builder()
        .doh(doh_server.url("/dns-query"))
        .tls(doh_server.tls())
        .keep_alive(false)
        .build()
        .unwrap();

    let url = format!("http://app.doh.test:{origin_port}/hello");
    assert_eq!(session.get(&url).text().await.unwrap(), "via-doh");
    let after_first = queries.load(Ordering::SeqCst);
    assert_eq!(after_first, 2, "expected one A + one AAAA query");

    assert_eq!(session.get(&url).text().await.unwrap(), "via-doh");
    assert_eq!(
        queries.load(Ordering::SeqCst),
        after_first,
        "second connection within TTL must hit the DoH cache"
    );

    for bad_host in ["missing.doh.test", "servfail.doh.test", "corrupt.doh.test"] {
        let err = session
            .get(format!("http://{bad_host}:{origin_port}/"))
            .send()
            .await
            .unwrap_err();
        assert_eq!(err.kind(), Kind::Dns, "{bad_host}: {err:?}");
    }
}

#[tokio::test]
async fn doh_auto_ech_fetches_https_record_and_encrypts_sni() {
    let (_hpke, ech_list, server_keys) = build_ech_config_list(11, "outer.doh.test");

    let (origin_cert, origin_key) =
        self_signed_dns_names(&["inner.doh.test", "outer.doh.test", "dns.local"]);
    let ca_pem = origin_cert.to_pem().unwrap();

    let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_port = origin_listener.local_addr().unwrap().port();
    let mut origin_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    origin_ctx
        .set_min_proto_version(Some(SslVersion::TLS1_3))
        .unwrap();
    origin_ctx.set_certificate(&origin_cert).unwrap();
    origin_ctx.set_private_key(&origin_key).unwrap();
    origin_ctx.set_ech_keys(&server_keys).unwrap();
    let origin_ctx = Arc::new(origin_ctx.build());

    tokio::spawn(async move {
        while let Ok((tcp, _)) = origin_listener.accept().await {
            let ctx = Arc::clone(&origin_ctx);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    let accepted = stream.ssl().ech_accepted();
                    let sni = stream
                        .ssl()
                        .servername(NameType::HOST_NAME)
                        .unwrap_or("none")
                        .to_owned();
                    serve_h1_ok(stream, &format!("auto-ech={accepted};sni={sni}")).await;
                }
            });
        }
    });

    let mut records: HashMap<(String, u16), Vec<u8>> = HashMap::new();
    records.insert(("inner.doh.test".into(), 1), vec![127, 0, 0, 1]);
    records.insert(
        (format!("_{origin_port}._https.inner.doh.test"), 65),
        svcb_ech_rdata(1, &ech_list),
    );
    let records = Arc::new(records);
    let https_queries = Arc::new(AtomicUsize::new(0));
    let https_queries_bg = Arc::clone(&https_queries);

    let doh_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let doh_addr: SocketAddr = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        doh_listener.local_addr().unwrap().port(),
    );
    let mut doh_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    doh_ctx.set_certificate(&origin_cert).unwrap();
    doh_ctx.set_private_key(&origin_key).unwrap();
    let doh_ctx = Arc::new(doh_ctx.build());

    tokio::spawn(async move {
        while let Ok((tcp, _)) = doh_listener.accept().await {
            let ctx = Arc::clone(&doh_ctx);
            let records = Arc::clone(&records);
            let https_queries_bg = Arc::clone(&https_queries_bg);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                let Ok(mut stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let Ok(n) = stream.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]);
                    let content_len: usize = head
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    let total = head_end + 4 + content_len;
                    if buf.len() < total {
                        continue;
                    }
                    let body = &buf[head_end + 4..total];
                    let Some((name, qtype)) = decode_qname_and_qtype(body) else {
                        return;
                    };
                    if qtype == 65 {
                        https_queries_bg.fetch_add(1, Ordering::SeqCst);
                    }
                    let wire = match records.get(&(name, qtype)) {
                        Some(rdata) => dns_response(body, 0, &[(qtype, 120, rdata)]),
                        None => dns_response(body, 0, &[]),
                    };
                    let resp_head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                        wire.len()
                    );
                    if stream.write_all(resp_head.as_bytes()).await.is_err()
                        || stream.write_all(&wire).await.is_err()
                    {
                        return;
                    }
                    buf.drain(..total);
                }
            });
        }
    });

    let session = Session::builder()
        .resolve("dns.local", [doh_addr])
        .doh(format!("https://dns.local:{}/dns-query", doh_addr.port()))
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_pem)
                .ech(true),
        )
        .keep_alive(false)
        .build()
        .unwrap();

    let url = format!("https://inner.doh.test:{origin_port}/auto-ech");
    let resp = session.get(&url).await.unwrap();
    assert!(resp.ech_accepted(), "expected automatic ECH over DoH");
    assert_eq!(
        resp.text().await.unwrap(),
        "auto-ech=true;sni=inner.doh.test"
    );
    assert_eq!(https_queries.load(Ordering::SeqCst), 1);

    let resp2 = session.get(&url).await.unwrap();
    assert!(resp2.ech_accepted());
    assert_eq!(
        https_queries.load(Ordering::SeqCst),
        1,
        "cached HTTPS ECH record must not re-query DoH within TTL"
    );
}

#[tokio::test]
async fn doh_does_not_leak_origin_pins_or_client_cert() {
    use sha2::{Digest, Sha256};

    let (origin_cert, origin_key) = super::tls_support::self_signed_dns("pinned.doh.test");
    let origin_pin: [u8; 32] = Sha256::digest(origin_cert.to_der().unwrap()).into();
    let (doh_cert, doh_key) = super::tls_support::self_signed_dns("dns.local");
    let (client_cert, client_key) = super::tls_support::self_signed_dns("client.local");
    let p12 = leyline_bssl::pkcs12::create("pw", "c", &client_key, &client_cert, None).unwrap();

    let mut ca_bundle = origin_cert.to_pem().unwrap();
    ca_bundle.extend_from_slice(&doh_cert.to_pem().unwrap());

    let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_port = origin_listener.local_addr().unwrap().port();
    let mut origin_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    origin_ctx.set_certificate(&origin_cert).unwrap();
    origin_ctx.set_private_key(&origin_key).unwrap();
    origin_ctx.cert_store_mut().add_cert(client_cert).unwrap();
    origin_ctx.set_verify(
        leyline_bssl::ssl::SslVerifyMode::PEER
            | leyline_bssl::ssl::SslVerifyMode::FAIL_IF_NO_PEER_CERT,
    );
    let origin_ctx = Arc::new(origin_ctx.build());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = origin_listener.accept().await {
            let ctx = Arc::clone(&origin_ctx);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    serve_h1_ok(stream, "origin-pinned-mtls").await;
                }
            });
        }
    });

    let doh_saw_client_cert = Arc::new(AtomicUsize::new(0));
    let doh_saw_bg = Arc::clone(&doh_saw_client_cert);
    let doh_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let doh_addr = doh_listener.local_addr().unwrap();
    let mut doh_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    doh_ctx.set_certificate(&doh_cert).unwrap();
    doh_ctx.set_private_key(&doh_key).unwrap();
    doh_ctx.set_verify(leyline_bssl::ssl::SslVerifyMode::PEER);
    let doh_ctx = Arc::new(doh_ctx.build());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = doh_listener.accept().await {
            let ctx = Arc::clone(&doh_ctx);
            let doh_saw_bg = Arc::clone(&doh_saw_bg);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                let Ok(mut stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                if stream.ssl().peer_certificate().is_some() {
                    doh_saw_bg.fetch_add(1, Ordering::SeqCst);
                }
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let Ok(n) = stream.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]);
                    let content_len: usize = head
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    let total = head_end + 4 + content_len;
                    if buf.len() < total {
                        continue;
                    }
                    let body = &buf[head_end + 4..total];
                    let Some((name, qtype)) = decode_qname_and_qtype(body) else {
                        return;
                    };
                    let wire = if name == "pinned.doh.test" && qtype == 1 {
                        dns_response(body, 0, &[(1, 120, &[127, 0, 0, 1])])
                    } else {
                        dns_response(body, 0, &[])
                    };
                    let resp_head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                        wire.len()
                    );
                    if stream.write_all(resp_head.as_bytes()).await.is_err()
                        || stream.write_all(&wire).await.is_err()
                    {
                        return;
                    }
                    buf.drain(..total);
                }
            });
        }
    });

    let session = Session::builder()
        .resolve("dns.local", [doh_addr])
        .doh(format!("https://dns.local:{}/dns-query", doh_addr.port()))
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_bundle)
                .pin_cert_sha256(origin_pin)
                .client_cert_pkcs12(p12, "pw"),
        )
        .build()
        .unwrap();

    let body = session
        .get(format!("https://pinned.doh.test:{origin_port}/secret"))
        .text()
        .await
        .unwrap();
    assert_eq!(body, "origin-pinned-mtls");
    assert_eq!(
        doh_saw_client_cert.load(Ordering::SeqCst),
        0,
        "origin mTLS client cert must never be presented to the DoH provider"
    );
}
