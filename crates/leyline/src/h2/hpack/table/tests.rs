use super::*;

/// `(&str, &str)` view of a dynamic-table entry for assertions.
fn entry(dt: &DynamicTable, i: usize) -> Option<(&str, &str)> {
    dt.get(i).map(|(n, v)| {
        (
            std::str::from_utf8(n).unwrap(),
            std::str::from_utf8(v).unwrap(),
        )
    })
}

fn b(s: &str) -> Bytes {
    Bytes::copy_from_slice(s.as_bytes())
}

#[test]
fn static_table_size() {
    assert_eq!(STATIC_TABLE.len(), 62); // 0-61
}

#[test]
fn static_table_common_entries() {
    assert_eq!(STATIC_TABLE[2], (":method", "GET"));
    assert_eq!(STATIC_TABLE[3], (":method", "POST"));
    assert_eq!(STATIC_TABLE[4], (":path", "/"));
    assert_eq!(STATIC_TABLE[7], (":scheme", "https"));
    assert_eq!(STATIC_TABLE[8], (":status", "200"));
}

#[test]
fn dynamic_table_insert_and_get() {
    let mut dt = DynamicTable::new();
    dt.insert(b("custom-header"), b("value1"));
    assert_eq!(entry(&dt, 0), Some(("custom-header", "value1")));
    assert_eq!(dt.len(), 1);
}

#[test]
fn dynamic_table_eviction() {
    // "aa" + "bb" + 32 = 36 bytes per entry. Max 70 = room for 1, not 2.
    let mut dt = DynamicTable::with_max_size(70);
    dt.insert(b("aa"), b("bb")); // 36 bytes
    assert_eq!(dt.len(), 1);

    dt.insert(b("cc"), b("dd")); // 36 bytes, total would be 72 > 70, evicts first
    assert_eq!(dt.len(), 1);
    assert_eq!(entry(&dt, 0), Some(("cc", "dd")));
}

#[test]
fn dynamic_table_oversized_entry_clears() {
    let mut dt = DynamicTable::with_max_size(32); // too small for any entry
    dt.insert(b("x"), b("y")); // 1+1+32 = 34, exceeds 32
    assert_eq!(dt.len(), 0);
}

#[test]
fn lookup_static_and_dynamic() {
    let mut dt = DynamicTable::new();
    dt.insert(b("x-custom"), b("val"));

    // Static lookup.
    let (n, v) = lookup(2, &dt).unwrap();
    assert_eq!(
        (n.as_ref(), v.as_ref()),
        (b"\x3amethod".as_ref(), b"GET".as_ref())
    );
    // Dynamic lookup (index 62 = first dynamic entry).
    let (n, v) = lookup(62, &dt).unwrap();
    assert_eq!(
        (n.as_ref(), v.as_ref()),
        (b"x-custom".as_ref(), b"val".as_ref())
    );
    // Out of range.
    assert!(lookup(63, &dt).is_none());
}

#[test]
fn find_static_exact() {
    assert_eq!(find_static(":method", "GET"), Some((2, true)));
    assert_eq!(find_static(":method", "POST"), Some((3, true)));
}

#[test]
fn find_static_name_only() {
    assert_eq!(find_static(":method", "DELETE"), Some((2, false)));
    assert_eq!(
        find_static("content-type", "application/json"),
        Some((31, false))
    );
}

#[test]
fn find_static_not_found() {
    assert_eq!(find_static("x-custom-header", ""), None);
}
