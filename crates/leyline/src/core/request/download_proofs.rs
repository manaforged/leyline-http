use super::{range_matches, strong_etag};

#[kani::proof]
#[kani::unwind(8)]
fn download_strong_etag_rejects_weak_and_ctl() {
    let buf: [u8; 6] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 6);
    let slice = &buf[..len];
    kani::assume(slice.iter().all(|b| b.is_ascii()));
    let s = std::str::from_utf8(slice).expect("ascii");
    let ok = strong_etag(s);
    if len < 2 || slice[0] != b'"' || slice[len - 1] != b'"' {
        assert!(!ok);
    } else {
        let inner = &slice[1..len - 1];
        let valid_inner = inner
            .iter()
            .all(|&b| b == 0x21 || (0x23..=0x7e).contains(&b));
        assert_eq!(ok, valid_inner);
    }
}

#[kani::proof]
fn download_content_range_requires_exact_offset_and_total() {
    let start: u64 = kani::any();
    let end: u64 = kani::any();
    let total: u64 = kani::any();
    let offset: u64 = kani::any();
    let got = range_matches(start, end, total, offset);
    if end == u64::MAX {
        assert_eq!(got, None);
    } else if start == offset && start <= end && end + 1 == total {
        assert_eq!(got, Some(total));
    } else {
        assert_eq!(got, None);
    }
}
