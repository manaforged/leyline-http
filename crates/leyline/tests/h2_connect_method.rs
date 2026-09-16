use leyline::h2::config::PseudoOrder;
use leyline::h2::connection::PseudoHeaders;
use leyline::h2::error::{ErrorCode, H2Error};

const CHROME_ORDER: [PseudoOrder; 4] = [
    PseudoOrder::Method,
    PseudoOrder::Authority,
    PseudoOrder::Scheme,
    PseudoOrder::Path,
];

fn names<'a>(list: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    list.iter().map(|(n, _)| *n).collect()
}

#[test]
fn classic_connect_omits_scheme_and_path() {
    let p = PseudoHeaders {
        method: "CONNECT".into(),
        scheme: "https".into(),
        authority: "example.com:443".into(),
        path: "/unused".into(),
        protocol: None,
    };
    let (list, len) = p.build_pseudo_list(&CHROME_ORDER).unwrap();
    let list = &list[..len];
    assert_eq!(names(list), vec![":method", ":authority"]);
    assert_eq!(list[0].1, "CONNECT");
    assert_eq!(list[1].1, "example.com:443");
}

#[test]
fn extended_connect_emits_protocol() {
    let p = PseudoHeaders {
        method: "CONNECT".into(),
        scheme: "https".into(),
        authority: "example.com:443".into(),
        path: "/chat".into(),
        protocol: Some("websocket".into()),
    };
    let (list, len) = p.build_pseudo_list(&CHROME_ORDER).unwrap();
    let list = &list[..len];
    assert_eq!(names(list), vec![":method", ":authority", ":protocol"]);
    assert_eq!(list[2].1, "websocket");
}

#[test]
fn non_connect_emits_all_four_pseudos() {
    let p = PseudoHeaders {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.com".into(),
        path: "/".into(),
        protocol: None,
    };
    let (list, len) = p.build_pseudo_list(&CHROME_ORDER).unwrap();
    let list = &list[..len];
    assert_eq!(
        names(list),
        vec![":method", ":authority", ":scheme", ":path"]
    );
}

#[test]
fn non_connect_with_protocol_still_appends_it() {
    let p = PseudoHeaders {
        method: "GET".into(),
        scheme: "https".into(),
        authority: "example.com".into(),
        path: "/".into(),
        protocol: Some("websocket".into()),
    };
    let (list, len) = p.build_pseudo_list(&CHROME_ORDER).unwrap();
    let list = &list[..len];
    assert_eq!(list.last().map(|(n, _)| *n), Some(":protocol"));
}

#[test]
fn connect_without_authority_rejected() {
    let p = PseudoHeaders {
        method: "CONNECT".into(),
        scheme: "https".into(),
        authority: "".into(),
        path: "/".into(),
        protocol: None,
    };
    let err = p.build_pseudo_list(&CHROME_ORDER).unwrap_err();
    match err {
        H2Error::Connection { code, .. } => assert_eq!(code, ErrorCode::ProtocolError),
        other => panic!("expected Connection error, got {other:?}"),
    }
}

#[test]
fn pseudo_order_honoured_for_non_connect() {
    let order = [
        PseudoOrder::Method,
        PseudoOrder::Path,
        PseudoOrder::Authority,
        PseudoOrder::Scheme,
    ];
    let p = PseudoHeaders {
        method: "POST".into(),
        scheme: "https".into(),
        authority: "example.com".into(),
        path: "/api".into(),
        protocol: None,
    };
    let (list, len) = p.build_pseudo_list(&order).unwrap();
    let list = &list[..len];
    assert_eq!(
        names(list),
        vec![":method", ":path", ":authority", ":scheme"]
    );
}
