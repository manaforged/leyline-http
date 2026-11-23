use std::io;

use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::ec::{EcGroup, EcKey};
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::nid::Nid;
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{SslAcceptor, SslConnector, SslMethod, SslVerifyMode, SslVersion};
use leyline_bssl::x509::extension::BasicConstraints;
use leyline_bssl::x509::{X509NameBuilder, X509};
use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream};

use crate::{accept, connect, RecordError, RecordStream, SslStream};

#[derive(Clone, Copy, Debug)]
enum StreamMode {
    SslTls12,
    SslTls13,
    RecordsTls13,
}

impl StreamMode {
    const ALL: [Self; 3] = [Self::SslTls12, Self::SslTls13, Self::RecordsTls13];

    fn version(self) -> SslVersion {
        match self {
            Self::SslTls12 => SslVersion::TLS1_2,
            Self::SslTls13 | Self::RecordsTls13 => SslVersion::TLS1_3,
        }
    }
}

enum ClientStream {
    Ssl(Box<SslStream<DuplexStream>>),
    Records(Box<RecordStream<DuplexStream>>),
}

impl ClientStream {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Ssl(s) => s.read(buf).await,
            Self::Records(s) => s.read(buf).await,
        }
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Ssl(s) => s.read_exact(buf).await,
            Self::Records(s) => s.read_exact(buf).await,
        }
    }

    async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            Self::Ssl(s) => s.write_all(buf).await,
            Self::Records(s) => s.write_all(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Ssl(s) => s.flush().await,
            Self::Records(s) => s.flush().await,
        }
    }
}

fn self_signed_cert() -> (X509, PKey<Private>) {
    let key = PKey::from_ec_key(
        EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
    )
    .unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    let mut serial = BigNum::new().unwrap();
    serial.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
    cert.set_serial_number(&serial.to_asn1_integer().unwrap())
        .unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    let bc = BasicConstraints::new().critical().ca().build().unwrap();
    cert.append_extension(&bc).unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    (cert.build(), key)
}

fn server_acceptor(version: SslVersion, require_client_cert: bool) -> SslAcceptor {
    let (cert, key) = self_signed_cert();
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    builder.set_certificate(&cert).unwrap();
    builder.set_private_key(&key).unwrap();
    builder.set_min_proto_version(Some(version)).unwrap();
    builder.set_max_proto_version(Some(version)).unwrap();
    if require_client_cert {
        builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    }
    builder.build()
}

fn client_connector(version: SslVersion) -> SslConnector {
    let mut builder = SslConnector::builder(SslMethod::tls()).unwrap();
    builder.set_verify(SslVerifyMode::NONE);
    builder.set_min_proto_version(Some(version)).unwrap();
    builder.set_max_proto_version(Some(version)).unwrap();
    builder.build()
}

async fn handshake_pair(mode: StreamMode) -> (ClientStream, SslStream<DuplexStream>) {
    let acceptor = server_acceptor(mode.version(), false);
    let connector = client_connector(mode.version());
    let (client_io, server_io) = duplex(64 * 1024);
    let mut config = connector.configure().unwrap();
    config.set_verify_hostname(false);

    let (client_res, server_res) = tokio::join!(
        connect(config, "localhost", client_io),
        accept(&acceptor, server_io)
    );
    let client = client_res.expect("client handshake");
    let server = server_res.expect("server handshake");
    let client = match mode {
        StreamMode::SslTls12 | StreamMode::SslTls13 => ClientStream::Ssl(Box::new(client)),
        StreamMode::RecordsTls13 => {
            ClientStream::Records(Box::new(client.into_records().expect("TLS 1.3 records")))
        }
    };
    (client, server)
}

fn is_record_error(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::InvalidData
        && err
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<RecordError>())
            .is_some()
}

#[tokio::test]
async fn ssl_stream_and_record_stream_normalize_corrupted_records_to_record_error() {
    for mode in StreamMode::ALL {
        let (mut client, mut server) = handshake_pair(mode).await;

        let (w, r) = tokio::join!(client.write_all(b"ping"), async {
            let mut buf = [0u8; 4];
            server.read_exact(&mut buf).await.unwrap();
            buf
        });
        w.unwrap();
        assert_eq!(&r, b"ping", "{mode:?}");

        let forged = [
            0x17, 0x03, 0x03, 0x00, 0x20, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
            0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
            0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
        ];
        server.get_mut().write_all(&forged).await.unwrap();
        server.get_mut().flush().await.unwrap();

        let mut out = [0u8; 16];
        let err = client
            .read(&mut out)
            .await
            .expect_err("forged ciphertext must fail");
        assert!(
            is_record_error(&err),
            "{mode:?}: expected RecordError InvalidData, got {err:?}"
        );
    }
}

#[tokio::test]
async fn tls13_post_handshake_mtls_alert_normalizes_on_both_ssl_and_record_streams() {
    for mode in [StreamMode::SslTls13, StreamMode::RecordsTls13] {
        let acceptor = server_acceptor(SslVersion::TLS1_3, true);
        let connector = client_connector(SslVersion::TLS1_3);
        let (client_io, server_io) = duplex(64 * 1024);
        let mut config = connector.configure().unwrap();
        config.set_verify_hostname(false);

        let server_task = tokio::spawn(async move { accept(&acceptor, server_io).await });
        let client_ssl = connect(config, "localhost", client_io)
            .await
            .expect("TLS 1.3 client finishes before server verifies empty cert");
        let mut client = match mode {
            StreamMode::SslTls13 => ClientStream::Ssl(Box::new(client_ssl)),
            StreamMode::RecordsTls13 => ClientStream::Records(Box::new(
                client_ssl.into_records().expect("TLS 1.3 records"),
            )),
            StreamMode::SslTls12 => unreachable!(),
        };

        let _ = client.write_all(b"GET / HTTP/1.1\r\n\r\n").await;
        let _ = client.flush().await;
        assert!(server_task.await.unwrap().is_err());

        let mut buf = [0u8; 64];
        let err = client
            .read(&mut buf)
            .await
            .expect_err("post-handshake certificate_required alert must fail");
        assert!(
            is_record_error(&err),
            "{mode:?}: expected RecordError InvalidData on post-handshake alert, got {err:?}"
        );
    }
}

#[tokio::test]
async fn ssl_stream_and_record_stream_treat_clean_close_notify_as_eof() {
    for mode in StreamMode::ALL {
        let (mut client, mut server) = handshake_pair(mode).await;

        let (server_res, client_res) = tokio::join!(
            async {
                server.write_all(b"bye").await.unwrap();
                server.shutdown().await.unwrap();
            },
            async {
                let mut head = [0u8; 3];
                client.read_exact(&mut head).await.unwrap();
                assert_eq!(&head, b"bye");
                let mut eof = [0u8; 8];
                client.read(&mut eof).await
            }
        );
        let () = server_res;
        assert_eq!(client_res.expect("clean EOF"), 0, "{mode:?}");
    }
}

#[tokio::test]
async fn ssl_stream_and_record_stream_never_classify_abrupt_eof_as_record_error() {
    for mode in StreamMode::ALL {
        let (mut client, server) = handshake_pair(mode).await;
        drop(server);

        let mut buf = [0u8; 16];
        let err = client
            .read(&mut buf)
            .await
            .expect_err("unclean TLS close without close_notify must error");
        assert_eq!(
            err.kind(),
            io::ErrorKind::UnexpectedEof,
            "{mode:?}: expected UnexpectedEof on abrupt close, got {err:?}"
        );
        assert!(
            !is_record_error(&err),
            "{mode:?}: abrupt EOF must not be classified as RecordError: {err:?}"
        );
    }
}

#[tokio::test]
async fn ssl_stream_and_record_stream_roundtrip_multi_record_payloads() {
    let payload: Vec<u8> = (0..65_536u32)
        .map(|i| (i.wrapping_mul(31) & 0xff) as u8)
        .collect();
    for mode in StreamMode::ALL {
        let (mut client, mut server) = handshake_pair(mode).await;
        let expected = payload.clone();

        let (sent, received) = tokio::join!(
            async {
                for chunk in payload.chunks(4099) {
                    client.write_all(chunk).await.unwrap();
                }
                client.flush().await.unwrap();
            },
            async {
                let mut out = vec![0u8; expected.len()];
                server.read_exact(&mut out).await.unwrap();
                out
            }
        );
        let () = sent;
        assert_eq!(received, expected, "{mode:?} client->server");

        let (sent, received) = tokio::join!(
            async {
                for chunk in payload.chunks(7919) {
                    server.write_all(chunk).await.unwrap();
                }
                server.flush().await.unwrap();
            },
            async {
                let mut out = vec![0u8; expected.len()];
                client.read_exact(&mut out).await.unwrap();
                out
            }
        );
        let () = sent;
        assert_eq!(received, expected, "{mode:?} server->client");
    }
}
