use super::*;
use crate::pool::types::LegKind;

async fn open(pooled: Option<u32>) -> Opened<u32> {
    let inflight = Inflight::<u32>::default();
    let key =
        crate::pool::Pool::new().key(Scheme::Https, "coalesced.test", 443, None, Carrier::Tcp);
    open_coalesced(
        &inflight,
        &key,
        Kind::Http2,
        || pooled.map(|conn| (conn, TlsInfo::default())),
        || async { Ok((2, TlsInfo::default())) },
    )
    .await
    .expect("open")
}

#[tokio::test]
async fn a_pooled_connection_is_reused_as_a_pooled_leg() {
    let opened = open(Some(1)).await;
    assert_eq!((opened.conn, opened.leg()), (1, LegKind::Pooled));
}

#[tokio::test]
async fn a_new_connection_is_a_fresh_leg() {
    let opened = open(None).await;
    assert_eq!((opened.conn, opened.leg()), (2, LegKind::Fresh));
}

#[test]
fn an_http1_answer_to_an_h2_offer_reports_the_negotiated_protocol() {
    for (alpn, negotiated) in [(None, "none"), (Some(&b"http/1.1"[..]), "http/1.1")] {
        let error = NotH2::of(alpn).error();
        assert_eq!(error.cause(), Kind::Http2);
        assert_eq!(
            error.message(),
            Some(format!("alpn: negotiated {negotiated}, expected h2").as_str())
        );
    }
}
