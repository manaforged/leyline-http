use super::record_size_update;

#[kani::proof]
fn multiple_table_size_updates_track_minimum_and_latest() {
    let s1: usize = kani::any();
    let s2: usize = kani::any();
    let s3: usize = kani::any();
    let mut pending = None;
    record_size_update(&mut pending, s1);
    record_size_update(&mut pending, s2);
    record_size_update(&mut pending, s3);
    assert_eq!(pending, Some((s1.min(s2).min(s3), s3)));
}
