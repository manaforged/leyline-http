use super::*;

#[test]
fn encode_indexed_method_get() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":method", "GET")]);
    assert_eq!(block, vec![0x82]);
}

#[test]
fn encode_indexed_path_root() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":path", "/")]);
    assert_eq!(block, vec![0x84]);
}

#[test]
fn encode_indexed_scheme_https() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":scheme", "https")]);
    assert_eq!(block, vec![0x87]);
}

#[test]
fn encode_literal_with_indexing() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[("custom-key", "custom-value")]);
    assert_eq!(block[0], 0x40);
    assert!(!block.is_empty());
    assert_eq!(enc.dynamic.len(), 1);
}

#[test]
fn second_request_uses_dynamic_table() {
    let mut enc = Encoder::new();

    let _block1 = enc.encode_header_block(&[("x-custom", "value1")]);
    assert_eq!(enc.dynamic.len(), 1);

    let block2 = enc.encode_header_block(&[("x-custom", "value2")]);
    assert_ne!(block2[0], 0x40);
}

#[test]
fn uppercase_name_encoded_as_lowercase() {
    let mut mixed = Encoder::new();
    let mixed_block = mixed.encode_header_block(&[("X-Extra", "val")]);

    let mut lower = Encoder::new();
    let lower_block = lower.encode_header_block(&[("x-extra", "val")]);

    assert_eq!(mixed_block, lower_block);
    assert_eq!(mixed.dynamic.get(0).unwrap().0, "x-extra");
}
