use super::*;
use crate::pool::types::LegKind;

async fn open(pooled: Option<u32>) -> Opened<u32> {
    let inflight = Inflight::<u32>::default();
    let key = crate::pool::Pool::new().key("https", "coalesced.test", 443, None, Transport::Tcp);
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
