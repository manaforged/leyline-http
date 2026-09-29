use super::*;

#[test]
fn root_store_is_non_empty() {
    let roots = load_system_roots().expect("open ROOT store");
    assert!(
        !roots.is_empty(),
        "Windows ROOT store returned zero certificates — bridge is broken"
    );
    for (i, der) in roots.iter().enumerate() {
        assert_eq!(der.first(), Some(&0x30), "root #{i} is not DER-encoded");
    }
}
