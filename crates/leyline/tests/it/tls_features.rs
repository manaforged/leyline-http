use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::{Kind, Session, Tls};
use leyline_bssl::ssl::{NameType, Ssl, SslContextBuilder, SslMethod, SslVerifyMode, SslVersion};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use super::tls_support::{
    HTTP11, build_ech_config_list, self_signed_dns, self_signed_dns_names, serve_h1_ok,
    tls_server_per_connection,
};

#[tokio::test]
async fn danger_accept_invalid_hostnames_preserves_ca_and_pin_checks() {
    let (cert, key) = self_signed_dns("wrong.example");
    let cert_der = cert.to_der().unwrap();
    let cert_pem = cert.to_pem().unwrap();
    let good_pin: [u8; 32] = Sha256::digest(&cert_der).into();
    let bad_pin = [0xAAu8; 32];

    let conns = Arc::new(AtomicUsize::new(0));
    let port = tls_server_per_connection(
        cert,
        key,
        conns,
        |_| HTTP11,
        |_, stream| async move {
            serve_h1_ok(stream, "hostname-skipped").await;
        },
    )
    .await;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let url = format!("https://expected.example:{port}/check");

    let strict = Session::builder()
        .resolve("expected.example", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(cert_pem.clone()),
        )
        .build()
        .unwrap();
    let err = strict.get(&url).send().await.unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Tls,
        "strict mode must reject hostname mismatch: {err:?}"
    );

    let skip_host = Session::builder()
        .resolve("expected.example", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(cert_pem.clone())
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap();
    let body = skip_host.get(&url).text().await.unwrap();
    assert_eq!(body, "hostname-skipped");

    let untrusted = Session::builder()
        .resolve("expected.example", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap();
    let err = untrusted.get(&url).send().await.unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Tls,
        "danger_accept_invalid_hostnames must still reject untrusted CAs: {err:?}"
    );

    let wrong_pin = Session::builder()
        .resolve("expected.example", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(cert_pem.clone())
                .pin_cert_sha256(bad_pin)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap();
    let err = wrong_pin.get(&url).send().await.unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Tls,
        "danger_accept_invalid_hostnames must still enforce pins: {err:?}"
    );

    let right_pin = Session::builder()
        .resolve("expected.example", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(cert_pem)
                .pin_cert_sha256(good_pin)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap();
    let body = right_pin.get(&url).text().await.unwrap();
    assert_eq!(body, "hostname-skipped");
}

#[tokio::test]
async fn client_cert_pkcs12_authenticates_mtls_and_masks_debug() {
    let (server_cert, server_key) = self_signed_dns("mtls.local");
    let server_ca = server_cert.to_pem().unwrap();
    let (client_cert, client_key) = self_signed_dns("client.local");
    let p12 =
        leyline_bssl::pkcs12::create("s3cr3t-p12-pass", "client", &client_key, &client_cert, None)
            .unwrap();

    let tls_cfg = Tls::new()
        .system_roots(false)
        .env_roots(false)
        .ca_pem(server_ca.clone())
        .client_cert_pkcs12(p12.clone(), "s3cr3t-p12-pass");
    let debug = format!("{tls_cfg:?}");
    assert!(
        !debug.contains("s3cr3t-p12-pass"),
        "pkcs12 password must be redacted in Debug: {debug}"
    );

    let bad_pass = Session::builder()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_ca.clone())
                .client_cert_pkcs12(p12.clone(), "wrong-pass"),
        )
        .build()
        .unwrap_err();
    assert_eq!(
        bad_pass.kind(),
        Kind::Config,
        "wrong pkcs12 password: {bad_pass:?}"
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    ctx.set_certificate(&server_cert).unwrap();
    ctx.set_private_key(&server_key).unwrap();
    ctx.cert_store_mut().add_cert(client_cert).unwrap();
    ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    let ctx = Arc::new(ctx.build());

    let seen_client_certs = Arc::new(AtomicUsize::new(0));
    let seen_bg = Arc::clone(&seen_client_certs);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let ctx = Arc::clone(&ctx);
            let seen_bg = Arc::clone(&seen_bg);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    if stream.ssl().peer_certificate().is_some() {
                        seen_bg.fetch_add(1, Ordering::SeqCst);
                    }
                    serve_h1_ok(stream, "mtls-ok").await;
                }
            });
        }
    });

    let url = format!("https://mtls.local:{}/secure", addr.port());
    let no_cert = Session::builder()
        .resolve("mtls.local", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_ca),
        )
        .build()
        .unwrap();
    let err = no_cert.get(&url).send().await.unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Tls | Kind::Io),
        "missing client cert must fail: {err:?}"
    );

    let authed = Session::builder()
        .resolve("mtls.local", [addr])
        .tls(tls_cfg)
        .build()
        .unwrap();
    let body = authed.get(&url).text().await.unwrap();
    assert_eq!(body, "mtls-ok");
    assert_eq!(seen_client_certs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ech_config_list_encrypts_sni_and_recovers_from_stale_config() {
    let (_server_hpke, live_list, server_keys) = build_ech_config_list(7, "public.outer.test");
    let (_stale_hpke, stale_list, _stale_keys) = build_ech_config_list(99, "public.outer.test");
    let (cert, key) = self_signed_dns_names(&["secret.inner.test", "public.outer.test"]);
    let ca_pem = cert.to_pem().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    ctx.set_min_proto_version(Some(SslVersion::TLS1_3)).unwrap();
    ctx.set_certificate(&cert).unwrap();
    ctx.set_private_key(&key).unwrap();
    ctx.set_ech_keys(&server_keys).unwrap();
    let ctx = Arc::new(ctx.build());

    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_bg = Arc::clone(&accepts);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            accepts_bg.fetch_add(1, Ordering::SeqCst);
            let ctx = Arc::clone(&ctx);
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
                    serve_h1_ok(stream, &format!("ech={accepted};sni={sni}")).await;
                }
            });
        }
    });

    let url = format!("https://secret.inner.test:{}/private", addr.port());

    let direct = Session::builder()
        .resolve("secret.inner.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_pem.clone())
                .ech_config_list(live_list),
        )
        .build()
        .unwrap();
    let resp = direct.get(&url).await.unwrap();
    assert!(resp.ech_accepted(), "expected ECH to be accepted");
    assert_eq!(resp.text().await.unwrap(), "ech=true;sni=secret.inner.test");
    assert_eq!(accepts.load(Ordering::SeqCst), 1);

    let retry_session = Session::builder()
        .resolve("secret.inner.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_pem.clone())
                .ech_config_list(stale_list),
        )
        .build()
        .unwrap();
    let resp = retry_session.get(&url).await.unwrap();
    assert!(
        resp.ech_accepted(),
        "stale ECH config must retry with server retry_configs and succeed"
    );
    assert_eq!(resp.text().await.unwrap(), "ech=true;sni=secret.inner.test");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        3,
        "stale ECH attempt + automatic retry must open two connections"
    );

    let malformed = Session::builder()
        .resolve("secret.inner.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_pem)
                .ech_config_list(vec![0x00, 0x01]),
        )
        .build()
        .unwrap();
    let err = malformed.get(&url).send().await.unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Config | Kind::Tls),
        "malformed ECHConfigList must fail cleanly: {err:?}"
    );
}

#[tokio::test]
async fn ech_retry_works_under_custom_verifier_with_distinct_outer_cert() {
    let (_live_hpke, _live_list, server_keys) = build_ech_config_list(12, "public.outer.test");
    let (_stale_hpke, stale_list, _stale_keys) = build_ech_config_list(88, "public.outer.test");
    let (inner_cert, inner_key) = self_signed_dns("secret.inner.test");
    let (outer_cert, outer_key) = self_signed_dns("public.outer.test");
    let inner_pin: [u8; 32] = Sha256::digest(inner_cert.to_der().unwrap()).into();
    let mut ca_bundle = inner_cert.to_pem().unwrap();
    ca_bundle.extend_from_slice(&outer_cert.to_pem().unwrap());

    let mut inner_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    inner_ctx
        .set_min_proto_version(Some(SslVersion::TLS1_3))
        .unwrap();
    inner_ctx.set_certificate(&inner_cert).unwrap();
    inner_ctx.set_private_key(&inner_key).unwrap();
    inner_ctx.set_ech_keys(&server_keys).unwrap();
    let inner_ctx = Arc::new(inner_ctx.build());

    let mut outer_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    outer_ctx
        .set_min_proto_version(Some(SslVersion::TLS1_3))
        .unwrap();
    outer_ctx.set_certificate(&outer_cert).unwrap();
    outer_ctx.set_private_key(&outer_key).unwrap();
    outer_ctx.set_ech_keys(&server_keys).unwrap();
    let outer_ctx = Arc::new(outer_ctx.build());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let conn_idx = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let idx = conn_idx.fetch_add(1, Ordering::SeqCst);
            let ctx = if idx == 0 {
                Arc::clone(&outer_ctx)
            } else {
                Arc::clone(&inner_ctx)
            };
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    let accepted = stream.ssl().ech_accepted();
                    serve_h1_ok(stream, &format!("ech={accepted}")).await;
                }
            });
        }
    });

    let session = Session::builder()
        .resolve("secret.inner.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_bundle)
                .pin_cert_sha256(inner_pin)
                .ech_config_list(stale_list),
        )
        .build()
        .unwrap();
    let resp = session
        .get(format!("https://secret.inner.test:{}/ok", addr.port()))
        .await
        .unwrap();
    assert!(resp.ech_accepted());
    assert_eq!(resp.text().await.unwrap(), "ech=true");
}

#[tokio::test]
async fn https_proxy_does_not_receive_origin_ech_config() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (_hpke, live_list, server_keys) = build_ech_config_list(15, "public.outer.test");
    let (origin_cert, origin_key) =
        self_signed_dns_names(&["secret.inner.test", "public.outer.test"]);
    let (proxy_cert, proxy_key) = self_signed_dns("proxy.local");
    let mut ca_bundle = origin_cert.to_pem().unwrap();
    ca_bundle.extend_from_slice(&proxy_cert.to_pem().unwrap());

    let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = origin_listener.local_addr().unwrap();
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
                    serve_h1_ok(stream, &format!("proxied-ech={accepted}")).await;
                }
            });
        }
    });

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let mut proxy_ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    proxy_ctx.set_certificate(&proxy_cert).unwrap();
    proxy_ctx.set_private_key(&proxy_key).unwrap();
    let proxy_ctx = Arc::new(proxy_ctx.build());
    let proxy_seen_sni: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let proxy_sni_slot = Arc::clone(&proxy_seen_sni);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = proxy_listener.accept().await {
            let ctx = Arc::clone(&proxy_ctx);
            let sni_slot = Arc::clone(&proxy_sni_slot);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                let Ok(mut client) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                *sni_slot.lock().unwrap() = client
                    .ssl()
                    .servername(leyline_bssl::ssl::NameType::HOST_NAME)
                    .map(str::to_owned);
                let mut buf = Vec::new();
                let mut tmp = [0u8; 512];
                loop {
                    let Ok(n) = client.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let mut upstream = tokio::net::TcpStream::connect(origin_addr).await.unwrap();
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await
                    .unwrap();
                drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
            });
        }
    });

    let session = Session::builder()
        .resolve("proxy.local", [proxy_addr])
        .proxy(format!("https://proxy.local:{}", proxy_addr.port()))
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(ca_bundle)
                .ech_config_list(live_list),
        )
        .build()
        .unwrap();
    let resp = session
        .get(format!(
            "https://secret.inner.test:{}/p",
            origin_addr.port()
        ))
        .await
        .unwrap();
    assert!(resp.ech_accepted());
    assert_eq!(resp.text().await.unwrap(), "proxied-ech=true");
    assert_eq!(
        proxy_seen_sni.lock().unwrap().as_deref(),
        Some("proxy.local"),
        "https proxy must receive its own plain SNI, never origin ECH outer SNI"
    );
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn h3_danger_accept_invalid_hostnames_preserves_ca_and_pin_checks() {
    use super::h3_support::{Limits, Reply, h3_server_with_tls};
    use leyline::{Browser, Protocol};
    use sha2::{Digest, Sha256};

    let (cert, key) = self_signed_dns("actual.h3.test");
    let pem = cert.to_pem().unwrap();
    let pin: [u8; 32] = Sha256::digest(cert.to_der().unwrap()).into();
    let server =
        h3_server_with_tls(cert, key, |_| {}, vec![Reply::Body(5)], Limits::default()).await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], server.port));
    let url = format!("https://wrong.h3.test:{}/", server.port);

    let base = || {
        Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http3)
            .retry(0)
            .resolve("wrong.h3.test", [addr])
    };

    let err = base()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(pem.clone()),
        )
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Http3 | Kind::Tls),
        "strict H3 must reject mismatched SAN: {err:?}"
    );

    let body = base()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(pem.clone())
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap()
        .get(&url)
        .text()
        .await
        .unwrap();
    assert_eq!(body, "xxxxx");

    let err = base()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Http3 | Kind::Tls),
        "H3 hostname bypass must still reject untrusted CA: {err:?}"
    );

    let err = base()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(pem.clone())
                .pin_cert_sha256([0x42; 32])
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Http3 | Kind::Tls),
        "H3 hostname bypass must still enforce pins: {err:?}"
    );

    let body = base()
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(pem)
                .pin_cert_sha256(pin)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap()
        .get(&url)
        .text()
        .await
        .unwrap();
    assert_eq!(body, "xxxxx");
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn h3_client_cert_pkcs12_authenticates_mtls() {
    use super::h3_support::{Limits, Reply, h3_server_with_tls};
    use leyline::{Browser, Protocol};
    use leyline_bssl::ssl::SslVerifyMode;

    let (server_cert, server_key) = self_signed_dns("mtls.h3.test");
    let server_pem = server_cert.to_pem().unwrap();
    let (client_cert, client_key) = self_signed_dns("client.h3.test");
    let p12 = leyline_bssl::pkcs12::create("h3-secret", "client", &client_key, &client_cert, None)
        .unwrap();

    let server = h3_server_with_tls(
        server_cert,
        server_key,
        move |ctx| {
            ctx.cert_store_mut().add_cert(client_cert).unwrap();
            ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        },
        vec![Reply::Body(4)],
        Limits::default(),
    )
    .await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], server.port));
    let url = format!("https://mtls.h3.test:{}/", server.port);

    let err = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .retry(0)
        .resolve("mtls.h3.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_pem.clone()),
        )
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Http3 | Kind::Tls),
        "H3 mTLS must fail without client cert: {err:?}"
    );

    let body = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .retry(0)
        .resolve("mtls.h3.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_pem)
                .client_cert_pkcs12(p12, "h3-secret"),
        )
        .build()
        .unwrap()
        .get(&url)
        .text()
        .await
        .unwrap();
    assert_eq!(body, "xxxx");
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn h3_ech_config_list_encrypts_sni_and_recovers_from_stale_config() {
    use super::h3_support::{Limits, Reply, h3_server_with_tls};
    use leyline::{Browser, Protocol};

    let (_live_hpke, live_list, server_keys) = build_ech_config_list(21, "outer.h3.test");
    let (_stale_hpke, stale_list, _stale_keys) = build_ech_config_list(91, "outer.h3.test");
    let (cert, key) = self_signed_dns_names(&["inner.h3.test", "outer.h3.test"]);
    let ca_pem = cert.to_pem().unwrap();

    let server = h3_server_with_tls(
        cert,
        key,
        move |ctx| {
            ctx.set_ech_keys(&server_keys).unwrap();
        },
        vec![Reply::Body(3)],
        Limits::default(),
    )
    .await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], server.port));
    let url = format!("https://inner.h3.test:{}/", server.port);

    for (label, list) in [("live", live_list.clone()), ("stale-retry", stale_list)] {
        let resp = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http3)
            .retry(0)
            .resolve("inner.h3.test", [addr])
            .tls(
                Tls::new()
                    .system_roots(false)
                    .env_roots(false)
                    .ca_pem(ca_pem.clone())
                    .ech_config_list(list),
            )
            .build()
            .unwrap()
            .get(&url)
            .await
            .unwrap_or_else(|e| panic!("H3 ECH ({label}) failed: {e:?}"));
        assert!(
            resp.ech_accepted(),
            "H3 ECH ({label}) must report ech_accepted"
        );
        assert_eq!(resp.text().await.unwrap(), "xxx");
    }

    let err = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .retry(0)
        .resolve("inner.h3.test", [addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ech_config_list(live_list),
        )
        .build()
        .unwrap()
        .get(&url)
        .send()
        .await
        .unwrap_err();
    assert!(
        matches!(err.kind(), Kind::Http3 | Kind::Tls),
        "H3 ECH with untrusted CA must return error cleanly without aborting: {err:?}"
    );
}
