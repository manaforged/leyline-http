pub(crate) const H2: &[u8] = b"h2";
pub(crate) const HTTP11: &[u8] = b"http/1.1";

const BROWSER: &[&[u8]] = &[H2, HTTP11];
const HTTP11_ONLY: &[&[u8]] = &[HTTP11];

const fn wire_len(protos: &[&[u8]]) -> usize {
    let mut len = 0;
    let mut i = 0;
    while i < protos.len() {
        len += 1 + protos[i].len();
        i += 1;
    }
    len
}

const fn wire<const N: usize>(protos: &[&[u8]]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut at = 0;
    let mut i = 0;
    while i < protos.len() {
        let proto = protos[i];
        assert!(proto.len() <= u8::MAX as usize);
        out[at] = proto.len() as u8;
        at += 1;
        let mut j = 0;
        while j < proto.len() {
            out[at] = proto[j];
            at += 1;
            j += 1;
        }
        i += 1;
    }
    out
}

const BROWSER_ARR: [u8; wire_len(BROWSER)] = wire(BROWSER);
const HTTP11_ARR: [u8; wire_len(HTTP11_ONLY)] = wire(HTTP11_ONLY);

pub(crate) const BROWSER_WIRE: &[u8] = &BROWSER_ARR;
pub(crate) const HTTP11_WIRE: &[u8] = &HTTP11_ARR;
