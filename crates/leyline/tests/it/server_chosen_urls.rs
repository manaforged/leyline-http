use leyline::testing::{TestResponse, TestServer};
use leyline::{Browser, Session};

#[tokio::test]
async fn a_next_page_on_another_origin_gets_no_session_token() {
    let other = TestServer::http(|_| TestResponse::new(200).close())
        .await
        .unwrap();
    let next = other.url("/page2");
    let first = TestServer::http(move |_| {
        TestResponse::new(200)
            .close()
            .header("link", format!("<{next}>; rel=\"next\""))
    })
    .await
    .unwrap();
    let session = Session::builder()
        .bearer_auth("session-token")
        .build()
        .unwrap();
    let mut pages = session.get(first.url("/page1")).pages();
    while let Some(page) = pages.next().await {
        page.unwrap();
    }
    assert_eq!(
        first.requests().await[0].header("authorization"),
        Some("Bearer session-token")
    );
    let seen = other.requests().await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].header("authorization"), None);
}

#[tokio::test]
async fn a_tab_follow_to_another_origin_drops_a_default_authorization() {
    let other = TestServer::http(|_| TestResponse::new(200).close())
        .await
        .unwrap();
    let first = TestServer::http(|_| TestResponse::new(200).close())
        .await
        .unwrap();
    let tab = Session::builder()
        .browser(Browser::default())
        .protocol(leyline::ProtocolPolicy::Http1)
        .headers([("authorization", "Bearer default-token")])
        .build()
        .unwrap()
        .tab();
    tab.open(first.url("/")).await.unwrap();
    tab.follow(other.url("/elsewhere")).await.unwrap();
    assert_eq!(
        first.requests().await[0].header("authorization"),
        Some("Bearer default-token")
    );
    assert_eq!(other.requests().await[0].header("authorization"), None);
}
