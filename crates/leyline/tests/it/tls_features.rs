use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::{Kind, Session, Tls};
use leyline_bssl::hpke::HpkeKey;
use leyline_bssl::ssl::{
    NameType, Ssl, SslContextBuilder, SslEchKeys, SslMethod, SslVerifyMode, SslVersion,
    marshal_ech_config,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::tls_support::{
    HTTP11, self_signed_dns, self_signed_dns_names, tls_server_per_connection,
};

async fn serve_h1_ok(mut stream: leyline_bssl_tokio::SslStream<tokio::net::TcpStream>, body: &str) {
    let mut buf = [0u8; 2048];
    if stream.read(&mut buf).await.is_err() {
        return;
    }
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    drop(stream.write_all(resp.as_bytes()).await);
    drop(stream.shutdown().await);
}

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

fn build_ech_config_list(config_id: u8, public_name: &str) -> (HpkeKey, Vec<u8>, SslEchKeys) {
    let key = HpkeKey::generate_x25519().unwrap();
    let single = marshal_ech_config(config_id, &key, public_name, 64).unwrap();
    let mut builder = SslEchKeys::builder().unwrap();
    builder.add(true, &single, &key).unwrap();
    let keys = builder.build();
    let list = keys.marshal_retry_configs().unwrap();
    (key, list, keys)
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
