use super::range_matches;

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
