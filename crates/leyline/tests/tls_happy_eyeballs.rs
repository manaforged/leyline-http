//! Integration test: exercise the public [`FingerprintConnector`]
//! builder surface around the new [`Resolver`] and
//! [`HappyEyeballsConfig`] knobs.
//!
//! Construction-only sanity checks — the per-address race logic has
//! thorough coverage in the crate's in-module tests, which exercise
//! the real [`happy_eyeballs_connect`] race against tokio listeners.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use leyline::profile::BrowserProfile;
use leyline::tcp::TcpProfile;
use leyline::tls::{
    FingerprintConnector, HappyEyeballsConfig, ResolveFuture, Resolver, SystemResolver,
};
use leyline::{Browser, Session, TlsTrustConfig};

/// Mock resolver that returns a fixed list.
struct StaticResolver(Vec<SocketAddr>);

impl Resolver for StaticResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let list = self.0.clone();
        Box::pin(async move { Ok(list) })
    }
}

/// Resolver that always errors.
struct FailingResolver;

impl Resolver for FailingResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        Box::pin(async move { Err(io::Error::other("boom")) })
    }
}

fn load_profile() -> BrowserProfile {
    let toml_str = include_str!("../profiles/chrome/146.toml");
    BrowserProfile::from_toml(toml_str).expect("chrome 146 profile should parse")
}

#[tokio::test]
async fn builder_accepts_custom_resolver() {
    let profile = load_profile();
    let resolver = Arc::new(StaticResolver(vec!["127.0.0.1:443".parse().unwrap()]));
    let _connector = FingerprintConnector::new(&profile, TcpProfile::LINUX, None)
        .expect("connector build")
        .with_resolver(resolver)
        .with_happy_eyeballs_config(HappyEyeballsConfig {
            resolve_delay: Duration::from_millis(50),
            attempt_limit: 4,
        });
}

#[tokio::test]
async fn builder_accepts_system_resolver() {
    let profile = load_profile();
    let _connector = FingerprintConnector::new(&profile, TcpProfile::LINUX, None)
        .expect("connector build")
        .with_resolver(Arc::new(SystemResolver));
}

#[tokio::test]
async fn default_happy_eyeballs_is_250ms() {
    let config = HappyEyeballsConfig::default();
    assert_eq!(config.resolve_delay, Duration::from_millis(250));
    assert_eq!(config.attempt_limit, 8);
}

#[tokio::test]
async fn failing_resolver_is_pluggable() {
    // Compile-time check: FailingResolver satisfies Resolver.
    let _: Arc<dyn Resolver> = Arc::new(FailingResolver);
}

#[test]
fn session_builder_exposes_dns_controls() {
    let resolver = Arc::new(StaticResolver(vec!["127.0.0.1:443".parse().unwrap()]));
    let _session = Session::builder()
        .browser(Browser::Chrome146)
        .resolver(resolver)
        .happy_eyeballs(HappyEyeballsConfig {
            resolve_delay: Duration::from_millis(25),
            attempt_limit: 2,
        })
        .build()
        .expect("session build");
}

#[test]
fn session_builder_exposes_trust_controls() {
    let _session = Session::builder()
        .browser(Browser::Chrome146)
        .tls_trust(
            TlsTrustConfig::new()
                .without_env_roots()
                .without_system_roots()
                .add_pinned_leaf_sha256([7; 32]),
        )
        .build()
        .expect("session build");
}

#[test]
fn invalid_der_root_is_rejected_at_build_time() {
    let err = Session::builder()
        .browser(Browser::Chrome146)
        .add_root_certificate_der([1, 2, 3, 4])
        .build()
        .expect_err("invalid DER CA should fail TLS setup");
    assert!(err.to_string().contains("ssl handshake"));
}
