use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use leyline::{Resolver, Session, Timeout, Tls};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

use super::{Fixed, Steady, hold, steady_traffic};
use crate::{Verdict, number};

const ROTATED_HOST: &str = "origin.test";
const REQUESTS_PER_ROTATED_CONNECTION: u32 = 20;

struct Authority {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    der: Vec<u8>,
}

fn authority() -> Authority {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "qualification authority");
    let key = rcgen::KeyPair::generate().expect("key");
    let der = params.self_signed(&key).expect("authority").der().to_vec();
    Authority { params, key, der }
}

fn leaf(authority: &Authority) -> TlsAcceptor {
    let key = rcgen::KeyPair::generate().expect("key");
    let params = rcgen::CertificateParams::new(vec![ROTATED_HOST.to_owned()]).expect("params");
    let issuer = rcgen::Issuer::from_params(&authority.params, &authority.key);
    let cert = params.signed_by(&key, &issuer).expect("leaf");
    let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], private)
        .expect("server config");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsAcceptor::from(Arc::new(config))
}

struct Rotating {
    current: AtomicUsize,
    acceptors: [TlsAcceptor; 3],
    handshakes: [AtomicU64; 3],
}

async fn serve_rotating(stream: TcpStream, rotating: Arc<Rotating>) {
    let generation = rotating.current.load(Ordering::Relaxed);
    let Ok(mut tls) = rotating.acceptors[generation].accept(stream).await else {
        return;
    };
    rotating.handshakes[generation].fetch_add(1, Ordering::Relaxed);
    let mut seen = Vec::new();
    for answered in 1..=REQUESTS_PER_ROTATED_CONNECTION {
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut more = [0u8; 1024];
            match tls.read(&mut more).await {
                Ok(read) if read > 0 => seen.extend_from_slice(&more[..read]),
                _ => return,
            }
        }
        seen.clear();
        let close = if answered == REQUESTS_PER_ROTATED_CONNECTION {
            "connection: close\r\n"
        } else {
            ""
        };
        let reply = format!("HTTP/1.1 200 OK\r\n{close}content-length: 2\r\n\r\nok");
        if tls.write_all(reply.as_bytes()).await.is_err() {
            return;
        }
    }
    drop(tls.shutdown().await);
}

pub async fn certificate_rotation(args: &[String], verdict: &Verdict) -> Value {
    let hold = hold(args);
    drop(rustls::crypto::aws_lc_rs::default_provider().install_default());
    let (trusted, stranger) = (authority(), authority());
    let rotating = Arc::new(Rotating {
        current: AtomicUsize::new(0),
        acceptors: [leaf(&trusted), leaf(&trusted), leaf(&stranger)],
        handshakes: std::array::from_fn(|_| AtomicU64::new(0)),
    });
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let address = listener.local_addr().expect("address");
    let accepting = Arc::clone(&rotating);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_rotating(stream, Arc::clone(&accepting)));
        }
    });
    let session = Session::builder()
        .dns_resolver(Arc::new(Fixed(vec![address])) as Arc<dyn Resolver>)
        .tls(
            Tls::new()
                .env_roots(false)
                .system_roots(false)
                .ca_der(trusted.der.clone()),
        )
        .timeout(Timeout::new().total(Duration::from_secs(5)))
        .build()
        .expect("session");
    let tally = Arc::new(Steady::default());
    let url = format!("https://{ROTATED_HOST}:{}/", address.port());
    let every = Duration::from_micros(number(args, "--every-us", 1000));
    let traffic = steady_traffic(session, url, every, Arc::clone(&tally));
    tokio::time::sleep(hold).await;
    rotating.current.store(1, Ordering::Relaxed);
    tokio::time::sleep(hold).await;
    let rotated = tally.snapshot();
    rotating.current.store(2, Ordering::Relaxed);
    tokio::time::sleep(hold).await;
    let refusing = tally.snapshot();
    tokio::time::sleep(hold).await;
    traffic.abort();
    let last = tally.snapshot();
    let shakes: Vec<u64> = rotating
        .handshakes
        .iter()
        .map(|h| h.load(Ordering::Relaxed))
        .collect();
    if rotated.1 > 0 {
        verdict.fail(format!(
            "{} requests failed across a rotation under the same authority, first: {:?}",
            rotated.1,
            tally.first_failure.lock().unwrap()
        ));
    }
    if shakes[0] == 0 || shakes[1] == 0 {
        verdict.fail(format!(
            "the run did not reach both certificates: {shakes:?}"
        ));
    }
    if shakes[2] > 0 {
        verdict.fail(format!(
            "{} handshakes completed with a certificate from an untrusted authority",
            shakes[2]
        ));
    }
    let accepted_late = last.0 - refusing.0;
    if accepted_late > 0 {
        verdict.fail(format!(
            "{accepted_late} requests succeeded {hold:?} after the certificate became untrusted"
        ));
    }
    json!({"good_before_stranger": rotated.0, "failed_before_stranger": rotated.1, "handshakes": shakes,
           "refused_after_stranger": last.1 - rotated.1, "accepted_late": accepted_late})
}
