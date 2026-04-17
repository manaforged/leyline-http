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

use leyline_profile::BrowserProfile;
use leyline_tcp::TcpProfile;
use leyline_tls::{
    FingerprintConnector, HappyEyeballsConfig, ResolveFuture, Resolver, SystemResolver,
};

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
    let toml_str = include_str!("../../../profiles/chrome/146.toml");
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
