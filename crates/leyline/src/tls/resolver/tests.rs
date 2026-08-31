use super::*;

/// A mock resolver that always returns the same static address list.
struct StaticResolver(pub Vec<SocketAddr>);

impl Resolver for StaticResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        let list = self.0.clone();
        Box::pin(async move { Ok(list) })
    }
}

/// A resolver that always errors out.
struct FailingResolver;

impl Resolver for FailingResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "resolver said no (test)",
            ))
        })
    }
}

#[tokio::test]
async fn static_resolver_returns_list() {
    let addr: SocketAddr = "127.0.0.1:443".parse().unwrap();
    let r = StaticResolver(vec![addr]);
    let out = r.resolve("example.test", 443).await.unwrap();
    assert_eq!(out, vec![addr]);
}

#[tokio::test]
async fn failing_resolver_errs() {
    let r = FailingResolver;
    let err = r.resolve("nope.test", 443).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[tokio::test]
async fn system_resolver_resolves_localhost() {
    let r = SystemResolver;
    let addrs = r.resolve("localhost", 443).await.unwrap();
    assert!(!addrs.is_empty());
    for a in &addrs {
        assert!(a.ip().is_loopback(), "{a:?} is not loopback");
    }
}
