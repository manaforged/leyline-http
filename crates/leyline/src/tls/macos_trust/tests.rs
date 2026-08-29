use super::*;

#[test]
fn root_store_is_non_empty_der() {
    let roots = load_system_roots().expect("read macOS trust anchors");
    assert!(!roots.is_empty(), "macOS system trust store is empty");
    for (index, der) in roots.iter().enumerate() {
        assert_eq!(der.first(), Some(&0x30), "root #{index} is not DER");
    }
}

#[test]
fn roots_are_deduplicated() {
    let roots = load_system_roots().expect("read macOS trust roots");
    let unique: HashSet<&Vec<u8>> = roots.iter().collect();
    assert_eq!(unique.len(), roots.len(), "duplicate roots in merged set");
}
