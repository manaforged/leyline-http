use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    marshal_ech_config, HandshakeError, Ssl, SslContext, SslEchKeys, SslMethod,
    SslSessionCacheMode, SslVerifyMode,
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
fn crypto_buffer_builder_handles_zero_and_initialized_writes() -> Check {
    let empty = CryptoBufferBuilder::with_capacity(0)?.build()?;
    // SAFETY: `empty` is a live CRYPTO_BUFFER returned by `build()`.
    unsafe {
        assert_eq!(ffi::CRYPTO_BUFFER_len(empty), 0);
        ffi::CRYPTO_BUFFER_free(empty);
    }

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
    assert!(!retry.is_empty());

    drop(mid);
    server.join().expect("server thread finishes");
    Ok(())
}

#[test]
fn panicking_custom_verify_callback_fails_handshake_without_aborting() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept client");
        let ctx = server_ctx(&cert, &key).expect("server ctx").build();
        let ssl = Ssl::new(&ctx).expect("server ssl");
        let _ = ssl.accept(sock);
    });

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

#[test]
fn records_open_rejects_short_slice_and_seal_rejects_oversized_plaintext() -> Check {
    let (cert, key) = self_signed()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept client");
        let ctx = server_ctx(&cert, &key).expect("server ctx").build();
        let ssl = Ssl::new(&ctx).expect("server ssl");
        let mut stream = ssl.accept(sock).expect("server accept");
        let mut buf = [0u8; 4];
        let _ = stream.read(&mut buf);
    });

    let sock = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let mut ctx = SslContext::builder(SslMethod::tls())?;
    ctx.set_verify(SslVerifyMode::NONE);
    let ssl = Ssl::new(&ctx.build())?;
    let stream = ssl.connect(sock)?;
    let mut records = Records::new(stream.ssl()).expect("TLS 1.3 records");

    let mut shorter_than_header = [0u8; HEADER_LEN - 1];
    assert!(records.open(&mut shorter_than_header).is_err());

    let mut shorter_than_tag = [0u8; HEADER_LEN + 16];
    assert!(records.open(&mut shorter_than_tag).is_err());

    let mut out = Vec::new();
    assert!(records
        .seal(APPLICATION_DATA, &vec![0u8; MAX_PLAINTEXT + 1], &mut out)
        .is_err());

    drop(stream);
    server.join().expect("server thread finishes");
    Ok(())
}
