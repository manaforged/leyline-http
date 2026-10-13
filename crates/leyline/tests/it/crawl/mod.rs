mod copies;
mod defaults;
mod links;
mod pace;
mod rules;
mod scope;
mod sitemaps;

pub(super) use std::collections::BTreeSet;
pub(super) use std::time::Duration;

pub(super) use leyline::testing::{TestResponse, TestServer};
pub(super) use leyline::{Respect, Scope, Session, SkipReason};

pub(super) fn html(body: &str) -> TestResponse {
    TestResponse::new(200)
        .header("content-type", "text/html; charset=utf-8")
        .body(body.to_owned())
}

pub(super) async fn targets(server: &TestServer) -> BTreeSet<String> {
    server
        .requests()
        .await
        .into_iter()
        .map(|request| request.target)
        .collect()
}

pub(super) async fn page_targets(server: &TestServer) -> BTreeSet<String> {
    let mut sent = targets(server).await;
    sent.remove("/robots.txt");
    sent.remove("/sitemap.xml");
    sent
}
