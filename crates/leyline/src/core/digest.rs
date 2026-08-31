//! HTTP Digest authentication (RFC 7616).

use md5::{Digest as Md5Digest, Md5};
use sha2::{Sha256, Sha512_256};

use crate::core::error::{Error, Result};

/// Digest credentials.
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
    hex::encode(bytes)
}

/// Parse a `WWW-Authenticate: Digest ...` challenge into its fields.
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
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key = &body[key_start..i];
        i += 1;
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
                        )));
                    }
                };
            }
            _ => {}
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
pub(crate) fn build_auth_header(
    challenge: &Challenge,
    auth: &DigestAuth,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> Option<String> {
    let qop = match challenge.qop.as_deref() {
        Some(offered) => pick_supported_qop(offered)?,
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
        alg.hash_hex(format!("{}:{}:{}", ha1, challenge.nonce, ha2).as_bytes())
    };

    let quoted = |s: &str| {
        let mut esc = String::with_capacity(s.len() + 2);
        esc.push('"');
        for c in s.chars() {
            if c == '"' || c == '\\' {
                esc.push('\\');
            }
            esc.push(c);
        }
        esc.push('"');
        esc
    };

    let mut out = format!(
        "Digest username={u}, realm={r}, nonce={n}, uri={uri}, algorithm={alg}, response={resp}",
        u = quoted(&auth.username),
        r = quoted(&challenge.realm),
        n = quoted(&challenge.nonce),
        uri = quoted(uri),
        alg = alg.wire_name(),
        resp = quoted(&response),
    );
    if !qop.is_empty() {
        out.push_str(&format!(
            ", qop={qop}, nc={nc_hex}, cnonce={}",
            quoted(cnonce)
        ));
    }
    if let Some(opaque) = &challenge.opaque {
        out.push_str(&format!(", opaque={}", quoted(opaque)));
    }
    Some(out)
}

/// Pick a supported `qop` token from the server's advertised list.
pub(crate) fn pick_supported_qop(qop: &str) -> Option<&'static str> {
    for token in qop.split(',') {
        if token.trim().eq_ignore_ascii_case("auth") {
            return Some("auth");
        }
    }
    None
}

/// Generate a fresh 16-hex-char client nonce.
pub(crate) fn generate_cnonce() -> String {
    crate::util::random_hex_token(8)
}

/// Monotonic nonce-count per server-nonce string, per process.
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

/// Test-only: reset the global nonce cache so unit tests don't leak counter state between runs.
#[cfg(test)]
pub(crate) fn reset_nonce_cache_for_test() {
    let mut guard = nonce_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
}

#[cfg(test)]
mod tests;
