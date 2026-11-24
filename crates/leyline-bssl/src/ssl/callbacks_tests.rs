use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::asn1::Asn1Time;
use crate::error::ErrorStack;
use crate::ffi;
use crate::hash::MessageDigest;
use crate::hpke::HpkeKey;
use crate::pkey::PKey;
use crate::records::{Records, APPLICATION_DATA, HEADER_LEN, MAX_PLAINTEXT};
use crate::rsa::Rsa;
use crate::ssl::{
    marshal_ech_config, CertificateCompressionAlgorithm, CertificateCompressor, HandshakeError,
    Ssl, SslContext, SslEchKeys, SslMethod, SslSessionCacheMode, SslVerifyMode,
};
use crate::x509::{X509Name, X509};

use super::CryptoBufferBuilder;

type Check = Result<(), Box<dyn std::error::Error>>;

fn self_signed() -> Result<(X509, PKey<crate::pkey::Private>), ErrorStack> {
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;
    let mut name = X509Name::builder()?;
    name.append_entry_by_text("CN", "localhost")?;
    let name = name.build();
    let mut builder = X509::builder()?;
    builder.set_version(2)?;
    builder.set_subject_name(&name)?;
    builder.set_issuer_name(&name)?;
    builder.set_pubkey(&key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(1)?;
    builder.set_not_before(&not_before)?;
    builder.set_not_after(&not_after)?;
    builder.sign(&key, MessageDigest::sha256())?;
    Ok((builder.build(), key))
}

fn server_ctx(
    cert: &X509,
    key: &PKey<crate::pkey::Private>,
) -> Result<crate::ssl::SslContextBuilder, ErrorStack> {
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_certificate(cert)?;
    ctx.set_private_key(key)?;
    Ok(ctx)
}

#[test]
fn crypto_buffer_builder_handles_zero_initialized_and_abandoned_writes() -> Check {
    let empty = CryptoBufferBuilder::with_capacity(0)?.build()?;
    // SAFETY: `empty` is a live CRYPTO_BUFFER returned by `build()`.
    unsafe {
        assert_eq!(ffi::CRYPTO_BUFFER_len(empty), 0);
        ffi::CRYPTO_BUFFER_free(empty);
    }

    let abandoned = CryptoBufferBuilder::with_capacity(64)?;
    drop(abandoned);

    let mut builder = CryptoBufferBuilder::with_capacity(5)?;
    builder.as_writer().write_all(b"hello")?;
    let filled = builder.build()?;
    // SAFETY: `filled` is a live CRYPTO_BUFFER of 5 initialized bytes.
    unsafe {
        let len = ffi::CRYPTO_BUFFER_len(filled);
        let bytes = std::slice::from_raw_parts(ffi::CRYPTO_BUFFER_data(filled), len);
        assert_eq!(bytes, b"hello");
        ffi::CRYPTO_BUFFER_free(filled);
    }
    Ok(())
}

#[test]
fn ech_retry_configs_requires_ech_rejected_error_stack() -> Check {
    let (cert, key) = self_signed()?;
    let server_hpke = HpkeKey::generate_x25519()?;
    let server_cfg = marshal_ech_config(1, &server_hpke, "public.example", 64)?;
    let mut ech_keys = SslEchKeys::builder()?;
    ech_keys.add(true, &server_cfg, &server_hpke)?;
    let ech_keys = ech_keys.build();
    let expected_retry = ech_keys.marshal_retry_configs()?;
    assert!(!expected_retry.is_empty());

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept client");
        let mut ctx = server_ctx(&cert, &key).expect("server ctx");
        ctx.set_ech_keys(&ech_keys).expect("set ech keys");
        let ssl = Ssl::new(&ctx.build()).expect("server ssl");
        let _ = ssl.accept(sock);
    });

    let stale_hpke = HpkeKey::generate_x25519()?;
    let stale_cfg = marshal_ech_config(2, &stale_hpke, "public.example", 64)?;
    let mut ech_list = Vec::with_capacity(2 + stale_cfg.len());
    ech_list.extend_from_slice(&u16::try_from(stale_cfg.len())?.to_be_bytes());
    ech_list.extend_from_slice(&stale_cfg);

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    let mut ssl = Ssl::new(&ctx.build())?;
    ssl.set_hostname("inner.example")?;
    ssl.set_ech_config_list(&ech_list)?;

    let err = ssl
        .connect(sock)
        .expect_err("stale ECH config must be rejected");
    let HandshakeError::Failure(mid) = err else {
        panic!("expected handshake failure on ECH rejection");
    };
    let stack = mid
        .error()
        .ssl_error()
        .expect("ECH rejection must carry an SSL error stack");
    assert!(
        mid.ssl().ech_retry_configs(&ErrorStack::get()).is_none(),
        "empty error stack must not expose ECH retry configs"
    );
    let retry = mid
        .ssl()
        .ech_retry_configs(stack)
        .expect("SSL_R_ECH_REJECTED error stack must return retry configs");
    assert_eq!(retry, expected_retry.as_slice());

    drop(mid);
    server.join().expect("server thread finishes");
    Ok(())
}

#[test]
fn verify_and_custom_verify_callbacks_handle_results_and_panics() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (sock, _) = listener.accept().expect("accept client");
            let ctx = server_ctx(&cert, &key).expect("server ctx").build();
            let ssl = Ssl::new(&ctx).expect("server ssl");
            let _ = ssl.accept(sock);
        }
    });

    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&calls);
    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify_callback(SslVerifyMode::PEER, move |preverify_ok, _| {
        seen.lock().expect("lock calls").push(preverify_ok);
        true
    });
    let ssl = Ssl::new(&ctx.build())?;
    let stream = ssl.connect(sock)?;
    let recorded = calls.lock().expect("lock calls").clone();
    assert_eq!(
        recorded.first().copied(),
        Some(false),
        "raw_verify must first pass preverify_ok=false for self-signed cert, got {recorded:?}"
    );
    drop(stream);

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify_callback(SslVerifyMode::PEER, |_, _| panic!("raw_verify boom"));
    let ssl = Ssl::new(&ctx.build())?;
    assert!(ssl.connect(sock).is_err());

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_custom_verify_callback(SslVerifyMode::PEER, |_| panic!("verify boom"));
    let ssl = Ssl::new(&ctx.build())?;
    assert!(ssl.connect(sock).is_err());

    server.join().expect("server thread finishes");
    Ok(())
}

#[test]
fn panicking_new_session_callback_does_not_double_free() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept client");
        let mut ctx = server_ctx(&cert, &key).expect("server ctx");
        ctx.set_session_cache_mode(SslSessionCacheMode::SERVER);
        let ssl = Ssl::new(&ctx.build()).expect("server ssl");
        let mut stream = ssl.accept(sock).expect("server accept");
        stream.write_all(b"ok").expect("server write");
        stream.flush().expect("server flush");
    });

    let invoked = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&invoked);
    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    ctx.set_session_cache_mode(SslSessionCacheMode::CLIENT);
    ctx.set_new_session_callback(move |_, _session| {
        seen.store(true, Ordering::SeqCst);
        panic!("new session boom");
    });
    let ssl = Ssl::new(&ctx.build())?;
    let mut stream = ssl.connect(sock)?;
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    assert!(
        invoked.load(Ordering::SeqCst),
        "new_session_callback must run on TLS 1.3 NewSessionTicket"
    );
    drop(stream);
    server.join().expect("server thread finishes");
    Ok(())
}

struct IdentityCompressor;
impl CertificateCompressor for IdentityCompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = true;
    const CAN_DECOMPRESS: bool = false;

    fn compress<W: Write>(&self, input: &[u8], output: &mut W) -> std::io::Result<()> {
        output.write_all(input)
    }
}

struct PanickingDecompressor(Arc<AtomicBool>);
impl CertificateCompressor for PanickingDecompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W: Write>(&self, _input: &[u8], _output: &mut W) -> std::io::Result<()> {
        self.0.store(true, Ordering::SeqCst);
        panic!("cert decompress boom");
    }
}

#[test]
fn alpn_select_cert_keylog_and_cert_decompress_panics_do_not_abort_or_leak() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let alpn_ran = Arc::new(AtomicBool::new(false));
    let select_cert_ran = Arc::new(AtomicBool::new(false));
    let alpn_flag = Arc::clone(&alpn_ran);
    let select_flag = Arc::clone(&select_cert_ran);
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept alpn+keylog client");
        let mut ctx = server_ctx(&cert, &key).expect("server ctx");
        ctx.set_alpn_select_callback(move |_, _| {
            alpn_flag.store(true, Ordering::SeqCst);
            panic!("alpn boom");
        });
        let ssl = Ssl::new(&ctx.build()).expect("server ssl");
        let _ = ssl.accept(sock);

        let (sock, _) = listener.accept().expect("accept select_cert client");
        let mut ctx = server_ctx(&cert, &key).expect("server ctx");
        ctx.set_select_certificate_callback(move |_| {
            select_flag.store(true, Ordering::SeqCst);
            panic!("select_cert boom");
        });
        let ssl = Ssl::new(&ctx.build()).expect("server ssl");
        let _ = ssl.accept(sock);

        let (sock, _) = listener.accept().expect("accept cert_decompress client");
        let mut ctx = server_ctx(&cert, &key).expect("server ctx");
        ctx.add_certificate_compression_algorithm(IdentityCompressor)
            .expect("register server compressor");
        let ssl = Ssl::new(&ctx.build()).expect("server ssl");
        let _ = ssl.accept(sock);
    });

    let keylog_ran = Arc::new(AtomicBool::new(false));
    let keylog_flag = Arc::clone(&keylog_ran);
    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    ctx.set_alpn_protos(b"\x02h2")?;
    ctx.set_keylog_callback(move |_, _| {
        keylog_flag.store(true, Ordering::SeqCst);
        panic!("keylog boom");
    });
    let ssl = Ssl::new(&ctx.build())?;
    let stream = ssl.connect(sock)?;
    assert!(stream.ssl().selected_alpn_protocol().is_none());
    drop(stream);

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    let ssl = Ssl::new(&ctx.build())?;
    assert!(ssl.connect(sock).is_err());

    let decompress_ran = Arc::new(AtomicBool::new(false));
    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    ctx.add_certificate_compression_algorithm(PanickingDecompressor(Arc::clone(&decompress_ran)))?;
    let ssl = Ssl::new(&ctx.build())?;
    assert!(ssl.connect(sock).is_err());

    server.join().expect("server thread finishes");
    assert!(alpn_ran.load(Ordering::SeqCst), "alpn callback must run");
    assert!(
        select_cert_ran.load(Ordering::SeqCst),
        "select_cert callback must run"
    );
    assert!(
        keylog_ran.load(Ordering::SeqCst),
        "keylog callback must run"
    );
    assert!(
        decompress_ran.load(Ordering::SeqCst),
        "cert decompress callback must run"
    );
    Ok(())
}

#[test]
fn records_enforce_length_bounds_and_advance_nonces() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept client");
        let ctx = server_ctx(&cert, &key).expect("server ctx").build();
        let ssl = Ssl::new(&ctx).expect("server ssl");
        let stream = ssl.accept(sock).expect("server accept");
        let records = Records::new(stream.ssl()).expect("server TLS 1.3 records");
        (stream, records)
    });

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    let ssl = Ssl::new(&ctx.build())?;
    let client_stream = ssl.connect(sock)?;
    let mut client_records = Records::new(client_stream.ssl()).expect("client TLS 1.3 records");
    let (server_stream, mut server_records) = server.join().expect("server thread finishes");

    let mut shorter_than_header = [0u8; HEADER_LEN - 1];
    assert!(client_records.open(&mut shorter_than_header).is_err());

    let mut shorter_than_tag = [0u8; HEADER_LEN + 16];
    assert!(client_records.open(&mut shorter_than_tag).is_err());

    let mut oversized_out = Vec::new();
    assert!(client_records
        .seal(
            APPLICATION_DATA,
            &vec![0u8; MAX_PLAINTEXT + 1],
            &mut oversized_out
        )
        .is_err());

    let mut empty_record = Vec::new();
    client_records.seal(APPLICATION_DATA, b"", &mut empty_record)?;
    assert_eq!(empty_record.len(), HEADER_LEN + 1 + 16);
    let (kind, range) = server_records.open(&mut empty_record)?;
    assert_eq!(kind, APPLICATION_DATA);
    assert!(empty_record[range].is_empty());

    let max_payload = vec![0x5au8; MAX_PLAINTEXT];
    let mut max_record = Vec::new();
    client_records.seal(APPLICATION_DATA, &max_payload, &mut max_record)?;
    let (kind, range) = server_records.open(&mut max_record)?;
    assert_eq!(kind, APPLICATION_DATA);
    assert_eq!(&max_record[range], max_payload.as_slice());

    let mut third_a = Vec::new();
    client_records.seal(APPLICATION_DATA, b"same", &mut third_a)?;
    let mut third_b = Vec::new();
    client_records.seal(APPLICATION_DATA, b"same", &mut third_b)?;
    assert_ne!(third_a, third_b, "sequence number must advance nonces");

    drop((client_stream, server_stream));
    Ok(())
}
