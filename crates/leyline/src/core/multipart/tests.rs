use super::*;

#[test]
fn boundaries_are_distinct_between_forms() {
    let a = Form::new();
    let b = Form::new();
    assert_ne!(a.boundary(), b.boundary());
    assert!(a.boundary().starts_with("----LeylineFormBoundary"));
}

#[test]
fn content_type_carries_boundary() {
    let f = Form::new();
    let ct = f.content_type();
    assert!(ct.starts_with("multipart/form-data; boundary=----LeylineFormBoundary"));
    assert!(ct.contains(f.boundary()));
}
