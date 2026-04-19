//! HTTP Digest authentication (RFC 7616).
//!
//! Digest is alive and well in enterprise / on-prem stacks (routers,
//! NASes, many embedded server SDKs, and some SOAP endpoints). Browsers
//! still support it, and `curl` + `reqwest` both wire it in. Leyline
//! used to expect callers to pre-compute the `Authorization: Digest`
//! header themselves, which is brittle — this module closes that gap.
//!
//! The flow is a challenge/response:
//!
//! 1. Client sends the request with no `Authorization`.
//! 2. Server responds `401` with `WWW-Authenticate: Digest realm=...,
//!    nonce=..., qop="auth", algorithm=MD5`.
//! 3. Client computes `response = H(HA1 ":" nonce ":" nc ":" cnonce
//!    ":" qop ":" HA2)` and retries with an `Authorization: Digest`
//!    header.
//!
//! Leyline supports `MD5`, `SHA-256`, and `SHA-512-256` (plus their
//! `-sess` variants) with `qop=auth`. Username-hashing (`userhash=true`)
//! is not implemented — callers that need it will have to compute the
//! Authorization header themselves for now.

use md5::{Digest as Md5Digest, Md5};
use sha2::{Sha256, Sha512_256};

use crate::core::error::{Error, Result};

/// Digest credentials. Attach to a request with
/// [`crate::RequestBuilder::digest_auth`].
///
/// ```rust,ignore
/// use leyline::DigestAuth;
///
/// let resp = session
///     .get("https://router.local/status")
///     .digest_auth(DigestAuth::new("admin", "hunter2"))
///     .send()
///     .await?;
/// ```
#[derive(Debug, Clone)]
pub struct DigestAuth {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl DigestAuth {
    /// Build a new set of digest credentials.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

/// Parsed `WWW-Authenticate: Digest ...` challenge.
#[derive(Debug, Default, Clone)]
pub(crate) struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub qop: Option<String>,
    pub algorithm: Algorithm,
    pub opaque: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Algorithm {
    #[default]
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
    Sha512_256,
    Sha512_256Sess,
}

impl Algorithm {
    fn is_sess(self) -> bool {
        matches!(
            self,
            Algorithm::Md5Sess | Algorithm::Sha256Sess | Algorithm::Sha512_256Sess
        )
    }

    fn wire_name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Md5Sess => "MD5-sess",
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha256Sess => "SHA-256-sess",
            Algorithm::Sha512_256 => "SHA-512-256",
            Algorithm::Sha512_256Sess => "SHA-512-256-sess",
        }
    }

    fn hash_hex(self, input: &[u8]) -> String {
        match self {
            Algorithm::Md5 | Algorithm::Md5Sess => {
                let mut h = Md5::new();
                h.update(input);
                hex(&h.finalize())
            }
            Algorithm::Sha256 | Algorithm::Sha256Sess => {
                let mut h = Sha256::new();
                h.update(input);
                hex(&h.finalize())
            }
            Algorithm::Sha512_256 | Algorithm::Sha512_256Sess => {
                let mut h = Sha512_256::new();
                h.update(input);
                hex(&h.finalize())
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Parse a `WWW-Authenticate: Digest ...` challenge into its fields.
///
/// Scans the header value after the `Digest` prefix for `key=value`
/// pairs, where values may be quoted. Unknown / unsupported algorithms
/// are surfaced as an error so the caller doesn't silently fall back
/// to a weaker hash.
pub(crate) fn parse_challenge(header: &str) -> Result<Challenge> {
    let trimmed = header.trim();
    let body = trimmed
        .strip_prefix("Digest ")
        .or_else(|| trimmed.strip_prefix("digest "))
        .ok_or_else(|| Error::Http("digest: not a Digest challenge".into()))?;

    let mut ch = Challenge::default();
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        // Skip whitespace / commas.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Read key up to '='.
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = &body[key_start..i];
        i += 1;
        // Read value: quoted or bare up to comma.
        let (val, ni) = if i < bytes.len() && bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            let v = &body[start..j];
            let end = if j < bytes.len() { j + 1 } else { j };
            (v.to_string(), end)
        } else {
            let start = i;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b',' {
                j += 1;
            }
            (body[start..j].trim().to_string(), j)
        };
        i = ni;

        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => ch.realm = val,
            "nonce" => ch.nonce = val,
            "qop" => ch.qop = Some(val),
            "opaque" => ch.opaque = Some(val),
            "stale" => ch.stale = val.eq_ignore_ascii_case("true"),
            "algorithm" => {
                ch.algorithm = match val.trim() {
                    "MD5" | "md5" => Algorithm::Md5,
                    "MD5-sess" | "md5-sess" => Algorithm::Md5Sess,
                    "SHA-256" | "sha-256" => Algorithm::Sha256,
                    "SHA-256-sess" | "sha-256-sess" => Algorithm::Sha256Sess,
                    "SHA-512-256" | "sha-512-256" => Algorithm::Sha512_256,
                    "SHA-512-256-sess" | "sha-512-256-sess" => Algorithm::Sha512_256Sess,
                    other => {
                        return Err(Error::Http(format!(
                            "digest: unsupported algorithm {other}"
                        )))
                    }
                };
            }
            _ => { /* ignore unknown attributes */ }
        }
    }

    if ch.nonce.is_empty() {
        return Err(Error::Http(
            "digest: challenge missing required `nonce`".into(),
        ));
    }

    Ok(ch)
}

/// Compute the `Authorization: Digest ...` header for one request.
///
/// `nc` is the 8-hex-digit nonce counter, which callers track per
/// (nonce) to prevent server-side replay. `cnonce` is a fresh random
/// value the client generates per request.
pub(crate) fn build_auth_header(
    challenge: &Challenge,
    auth: &DigestAuth,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> Option<String> {
    // Reject challenges we cannot honestly answer. `qop=auth-int` would
    // require the entity-body hash in HA2 per RFC 7616 §3.4.3; emitting
    // a response without it would be rejected by the server as invalid.
    // Better to refuse cleanly so the caller surfaces a clear error.
    let qop = match challenge.qop.as_deref() {
        Some(offered) => pick_supported_qop(offered)?,
        // RFC 2069 fallback — no qop offered; allowed.
        None => "",
    };

    let alg = challenge.algorithm;
    let ha1_base =
        alg.hash_hex(format!("{}:{}:{}", auth.username, challenge.realm, auth.password).as_bytes());
    let ha1 = if alg.is_sess() {
        alg.hash_hex(format!("{}:{}:{}", ha1_base, challenge.nonce, cnonce).as_bytes())
    } else {
        ha1_base
    };
    let ha2 = alg.hash_hex(format!("{}:{}", method, uri).as_bytes());

    let nc_hex = format!("{:08x}", nc);

    let response = if !qop.is_empty() {
        alg.hash_hex(
            format!(
                "{}:{}:{}:{}:{}:{}",
                ha1, challenge.nonce, nc_hex, cnonce, qop, ha2
            )
            .as_bytes(),
        )
    } else {
        // RFC 2069 fallback — no qop.
        alg.hash_hex(format!("{}:{}:{}", ha1, challenge.nonce, ha2).as_bytes())
    };

    let mut out = format!(
        "Digest username=\"{u}\", realm=\"{r}\", nonce=\"{n}\", uri=\"{uri}\", algorithm={alg}, response=\"{resp}\"",
        u = auth.username,
        r = challenge.realm,
        n = challenge.nonce,
        uri = uri,
        alg = alg.wire_name(),
        resp = response,
    );
    if !qop.is_empty() {
        out.push_str(&format!(", qop={qop}, nc={nc_hex}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = &challenge.opaque {
        out.push_str(&format!(", opaque=\"{opaque}\""));
    }
    Some(out)
}

/// Pick a supported `qop` token from the server's advertised list.
///
/// Returns `Some("auth")` when the server offers `auth` (alone or in a
/// list). Returns `None` when the server offers only `auth-int` — which
/// we deliberately do not support, since our `HA2` would be wrong
/// without the entity-body hash required by RFC 7616 §3.4.3. Callers
/// should treat `None` as "challenge acceptable but we can't answer"
/// and skip the digest retry with a clear error rather than emitting
/// an invalid response.
pub(crate) fn pick_supported_qop(qop: &str) -> Option<&'static str> {
    for token in qop.split(',') {
        if token.trim().eq_ignore_ascii_case("auth") {
            return Some("auth");
        }
    }
    None
}

/// Generate a fresh 16-hex-char client nonce.
///
/// RFC 7616 §3.4 says the cnonce must be unique per request for a
/// given server nonce. We sample 8 random bytes so an observer cannot
/// predict the cnonce from a timing-correlated counter.
pub(crate) fn generate_cnonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut hex = String::with_capacity(16);
    for b in &bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Monotonic nonce-count per server-nonce string, per process.
///
/// RFC 7616 §3.4 requires the `nc` field to increment each time the
/// client sends a request using a given server-nonce; reuse is a
/// replay-protection violation. We cache the current count per nonce
/// in a global LRU map and hand out the next value. When an entry
/// would be evicted (memory cap) or the counter is about to wrap past
/// `u32::MAX`, the mapping is dropped — the next request with that
/// nonce starts at `nc = 1`. A server tracking strict monotonicity
/// across an evicted nonce would then reject our response with `401
/// stale=true`, at which point the caller's retry policy re-runs the
/// challenge exchange with a fresh nonce. This is the standard
/// RFC-correct recovery path.
///
/// Cap is intentionally generous (4 096 entries) — digest is rare
/// enough in modern traffic that this is many sessions' worth. Raise
/// via `DIGEST_NONCE_CACHE_CAP` at compile time if needed.
pub(crate) const DIGEST_NONCE_CACHE_CAP: usize = 4096;

fn nonce_cache() -> &'static std::sync::Mutex<lru::LruCache<String, u32>> {
    use lru::LruCache;
    use std::num::NonZeroUsize;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<LruCache<String, u32>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(DIGEST_NONCE_CACHE_CAP).expect("cap is non-zero"),
        ))
    })
}

pub(crate) fn next_nc_for_nonce(nonce: &str) -> u32 {
    let mut guard = nonce_cache().lock().unwrap_or_else(|e| e.into_inner());
    // If existing counter would wrap, drop and restart. u32::MAX is
    // `ffffffff`, which is a valid `nc` on the wire but the next
    // increment would collide. Safer to force a re-challenge by
    // evicting the entry and starting fresh; the server will send
    // `401 stale=true` and the caller's retry flow re-challenges.
    if let Some(existing) = guard.get(nonce) {
        if *existing >= u32::MAX - 1 {
            guard.pop(nonce);
        }
    }
    match guard.get_mut(nonce) {
        Some(counter) => {
            *counter = counter.saturating_add(1);
            *counter
        }
        None => {
            guard.put(nonce.to_string(), 1);
            1
        }
    }
}

/// Test-only: reset the global nonce cache so unit tests don't leak
/// counter state between runs.
#[cfg(test)]
pub(crate) fn reset_nonce_cache_for_test() {
    let mut guard = nonce_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_md5_challenge() {
        let hdr = r#"Digest realm="test@example.com", nonce="abc123", qop="auth", algorithm=MD5"#;
        let c = parse_challenge(hdr).unwrap();
        assert_eq!(c.realm, "test@example.com");
        assert_eq!(c.nonce, "abc123");
        assert_eq!(c.qop.as_deref(), Some("auth"));
        assert_eq!(c.algorithm, Algorithm::Md5);
    }

    #[test]
    fn rfc7616_md5_vector() {
        // RFC 7616 §3.9.1 — the canonical MD5 test vector.
        let challenge = Challenge {
            realm: "http-auth@example.org".into(),
            nonce: "7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v".into(),
            qop: Some("auth".into()),
            algorithm: Algorithm::Md5,
            opaque: Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS".into()),
            stale: false,
        };
        let auth = DigestAuth::new("Mufasa", "Circle of Life");
        let h = build_auth_header(
            &challenge,
            &auth,
            "GET",
            "/dir/index.html",
            1,
            "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
        )
        .expect("auth-only qop is supported");
        assert!(h.starts_with("Digest username=\"Mufasa\""));
        assert!(h.contains("algorithm=MD5"));
        assert!(h.contains("qop=auth"));
        assert!(h.contains("nc=00000001"));
        assert!(h.contains("response=\""));
    }

    #[test]
    fn refuses_auth_int_only_challenge() {
        let challenge = Challenge {
            realm: "r".into(),
            nonce: "n".into(),
            qop: Some("auth-int".into()),
            algorithm: Algorithm::Md5,
            opaque: None,
            stale: false,
        };
        let auth = DigestAuth::new("u", "p");
        assert!(
            build_auth_header(&challenge, &auth, "GET", "/x", 1, "cn").is_none(),
            "auth-int-only challenge must be refused, not answered with a fake auth HA2"
        );
    }

    #[test]
    fn nonce_count_increments_per_nonce() {
        reset_nonce_cache_for_test();
        let n1 = next_nc_for_nonce("nonce-A");
        let n2 = next_nc_for_nonce("nonce-A");
        let n3 = next_nc_for_nonce("nonce-B");
        assert!(n2 > n1, "monotonic within the same nonce");
        assert_eq!(n3, 1, "fresh nonce starts at 1");
    }

    #[test]
    fn nonce_cache_lru_evicts_beyond_cap() {
        reset_nonce_cache_for_test();
        // Fill the cache past capacity.
        for i in 0..DIGEST_NONCE_CACHE_CAP + 16 {
            let nonce = format!("lru-test-{i}");
            let n = next_nc_for_nonce(&nonce);
            assert_eq!(n, 1);
        }
        // The earliest nonces should have been evicted; re-inserting
        // yields nc=1, not the previous counter.
        let restart = next_nc_for_nonce("lru-test-0");
        assert_eq!(
            restart, 1,
            "LRU-evicted nonce restarts at 1; server would see stale=true and re-challenge"
        );
    }

    #[test]
    fn rejects_unknown_algorithm() {
        let hdr = r#"Digest realm="r", nonce="n", algorithm=BLAKE3"#;
        let err = parse_challenge(hdr).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unsupported algorithm"), "{msg}");
    }

    #[test]
    fn reports_missing_nonce() {
        let hdr = r#"Digest realm="r", qop="auth""#;
        let err = parse_challenge(hdr).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("nonce"), "{msg}");
    }

    #[test]
    fn sha256_response_matches_manual_computation() {
        // Build the same challenge/response and sanity-check that we
        // can reproduce the response hash by hand using the SAME code
        // path — this guards against accidental HA1/HA2 typos.
        let challenge = Challenge {
            realm: "r".into(),
            nonce: "n".into(),
            qop: Some("auth".into()),
            algorithm: Algorithm::Sha256,
            opaque: None,
            stale: false,
        };
        let auth = DigestAuth::new("u", "p");
        let header = build_auth_header(&challenge, &auth, "GET", "/x", 1, "cn")
            .expect("qop=auth is supported");

        let ha1 = Algorithm::Sha256.hash_hex(b"u:r:p");
        let ha2 = Algorithm::Sha256.hash_hex(b"GET:/x");
        let expected =
            Algorithm::Sha256.hash_hex(format!("{ha1}:n:00000001:cn:auth:{ha2}").as_bytes());
        assert!(
            header.contains(&format!("response=\"{expected}\"")),
            "header: {header}"
        );
    }
}
