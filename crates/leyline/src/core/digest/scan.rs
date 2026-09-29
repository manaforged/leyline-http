pub(super) struct Pair<'a> {
    pub key: &'a str,
    pub val: String,
}

pub(super) fn pairs(body: &str) -> Vec<Pair<'_>> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = &body[start..i];
        i += 1;
        let (val, next) = value(body, i);
        i = next;
        out.push(Pair { key, val });
    }
    out
}

fn value(body: &str, i: usize) -> (String, usize) {
    let bytes = body.as_bytes();
    if i < bytes.len() && bytes[i] == b'"' {
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j] != b'"' {
            j += 1;
        }
        let end = if j < bytes.len() { j + 1 } else { j };
        (body[start..j].to_string(), end)
    } else {
        let start = i;
        let mut j = start;
        while j < bytes.len() && bytes[j] != b',' {
            j += 1;
        }
        (body[start..j].trim().to_string(), j)
    }
}
