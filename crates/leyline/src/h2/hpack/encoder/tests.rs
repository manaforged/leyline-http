use super::*;

#[test]
fn encode_indexed_method_get() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":method", "GET")]);
    // :method GET is static index 2 → 0x82
    assert_eq!(block, vec![0x82]);
}

#[test]
fn encode_indexed_path_root() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":path", "/")]);
    // :path / is static index 4 → 0x84
    assert_eq!(block, vec![0x84]);
}

#[test]
fn encode_indexed_scheme_https() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":scheme", "https")]);
    // :scheme https is static index 7 → 0x87
    assert_eq!(block, vec![0x87]);
}

#[test]
fn encode_literal_with_indexing() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[("custom-key", "custom-value")]);
    // New name (0x40), then name string, then value string.
    assert_eq!(block[0], 0x40);
    assert!(!block.is_empty());
    // After encoding, the entry should be in the dynamic table.
    assert_eq!(enc.dynamic.len(), 1);
}

#[test]
fn second_request_uses_dynamic_table() {
    let mut enc = Encoder::new();

    // First request — new entry added to dynamic table.
    let _block1 = enc.encode_header_block(&[("x-custom", "value1")]);
    assert_eq!(enc.dynamic.len(), 1);

    // Second request — same name, different value. Should reference dynamic table name.
    let block2 = enc.encode_header_block(&[("x-custom", "value2")]);
    // Should use literal with indexed name (0x40 | index), not new name (0x40 | 0).
    assert_ne!(block2[0], 0x40); // not a new-name literal
}

#[test]
fn uppercase_name_encoded_as_lowercase() {
    // RFC 7540 §8.1.2 — HTTP/2 header names must be lowercase. A request
    // with uppercase names is malformed and the peer MUST treat it as a
    // stream error. Some third-party SDK headers arrive in mixed case;
    // this encoder must normalise them regardless of caller hygiene.
    let mut mixed = Encoder::new();
    let mixed_block = mixed.encode_header_block(&[("X-Extra", "val")]);

    let mut lower = Encoder::new();
    let lower_block = lower.encode_header_block(&[("x-extra", "val")]);

    assert_eq!(mixed_block, lower_block);
    assert_eq!(mixed.dynamic.get(0).unwrap().0, "x-extra");
}
