use super::*;
use futures_util::stream;

#[test]
fn from_vec_keeps_len_hint() {
    let b: Body = vec![1u8, 2, 3].into();
    assert_eq!(b.len_hint(), Some(3));
    assert!(!b.is_empty());
    assert!(!b.is_stream());
}

#[test]
fn empty_cases() {
    let b: Body = Body::default();
    assert!(b.is_empty());
    assert_eq!(b.len_hint(), Some(0));

    let b: Body = vec![].into();
    assert!(b.is_empty());

    let b: Body = "".into();
    assert!(b.is_empty());
}

#[test]
fn stream_without_length() {
    let s = stream::iter(vec![Ok::<_, io::Error>(Bytes::from_static(b"abc"))]);
    let b = Body::stream(s);
    assert!(b.is_stream());
    assert_eq!(b.len_hint(), None);
}

#[test]
fn stream_with_length() {
    let s = stream::iter(vec![Ok::<_, io::Error>(Bytes::from_static(b"abc"))]);
    let b = Body::stream_with_length(s, 3);
    assert_eq!(b.len_hint(), Some(3));
}
