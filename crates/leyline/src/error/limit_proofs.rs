use super::BodyLimit;

#[kani::proof]
fn a_body_limit_admits_exactly_what_fits() {
    let limit = BodyLimit::session(kani::any());
    let (seen, more): (usize, usize) = (kani::any(), kani::any());
    let fits = u128::from(seen as u64) + u128::from(more as u64) <= u128::from(limit.bytes as u64);
    let unlimited = limit.bytes == usize::MAX;
    assert_eq!(limit.admit(seen, more).is_ok(), fits || unlimited);
}

#[kani::proof]
fn a_body_limit_room_is_exactly_what_is_left() {
    let limit = BodyLimit::session(kani::any_where(|bytes| *bytes < usize::MAX));
    let used: usize = kani::any_where(|used| *used <= limit.bytes);
    let room = limit.room(used);
    assert_eq!(used + room, limit.bytes);
    assert!(limit.admit(used, room).is_ok());
    assert!(limit.admit(used, room + 1).is_err());
}
