use md5::{Digest as Md5Digest, Md5};
use sha2::{Sha256, Sha512_256};

use crate::core::config::Origin;
use crate::error::{Error, Kind, Result};
use crate::util::quote;

mod scan;

use scan::{Pair, pairs};

#[derive(Clone)]
pub(crate) struct DigestAuth {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl std::fmt::Debug for DigestAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DigestAuth")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl DigestAuth {
    pub(crate) fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub qop: Option<String>,
    pub algorithm: Algorithm,
    pub opaque: Option<String>,
    pub stale: bool,
    pub domain: Vec<String>,
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
    const ALL: [Algorithm; 6] = [
        Algorithm::Md5,
        Algorithm::Md5Sess,
        Algorithm::Sha256,
        Algorithm::Sha256Sess,
        Algorithm::Sha512_256,
        Algorithm::Sha512_256Sess,
    ];

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
                hex::encode(h.finalize())
            }
            Algorithm::Sha256 | Algorithm::Sha256Sess => {
                let mut h = Sha256::new();
                h.update(input);
                hex::encode(h.finalize())
            }
            Algorithm::Sha512_256 | Algorithm::Sha512_256Sess => {
                let mut h = Sha512_256::new();
                h.update(input);
                hex::encode(h.finalize())
            }
        }
    }
}

const SCHEME: &str = "Digest ";

pub(crate) fn parse_challenge(header: &str) -> Result<Challenge> {
    let trimmed = header.trim();
    let body = trimmed
        .get(..SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
        .map(|_| &trimmed[SCHEME.len()..])
        .ok_or_else(|| Error::new(Kind::Request).with_message("digest: not a Digest challenge"))?;

    let mut ch = Challenge::default();
    for pair in pairs(body) {
        ch.fill(pair)?;
    }

    if ch.nonce.is_empty() {
        return Err(
            Error::new(Kind::Request).with_message("digest: challenge missing required `nonce`")
        );
    }

    Ok(ch)
}

impl Challenge {
    fn fill(&mut self, pair: Pair<'_>) -> Result<()> {
        let Pair { key, val } = pair;
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => self.realm = val,
            "nonce" => self.nonce = val,
            "qop" => self.qop = Some(val),
            "opaque" => self.opaque = Some(val),
            "stale" => self.stale = val.eq_ignore_ascii_case("true"),
            "algorithm" => self.algorithm = algorithm(val.trim())?,
            "domain" => self.domain = val.split_whitespace().map(str::to_owned).collect(),
            _ => {}
        }
        Ok(())
    }
}

fn algorithm(val: &str) -> Result<Algorithm> {
    Algorithm::ALL
        .into_iter()
        .find(|alg| alg.wire_name().eq_ignore_ascii_case(val))
        .ok_or_else(|| {
            Error::new(Kind::Request).with_message(format!("digest: unsupported algorithm {val}"))
        })
}

impl Challenge {
    pub(crate) fn build_auth_header(
        &self,
        auth: &DigestAuth,
        method: &str,
        uri: &str,
        nc: u32,
        cnonce: &str,
    ) -> Option<String> {
        let qop = match self.qop.as_deref() {
            Some(offered) => pick_supported_qop(offered)?,
            None => "",
        };

        let alg = self.algorithm;
        let ha1_base =
            alg.hash_hex(format!("{}:{}:{}", auth.username, self.realm, auth.password).as_bytes());
        let ha1 = if alg.is_sess() {
            alg.hash_hex(format!("{}:{}:{}", ha1_base, self.nonce, cnonce).as_bytes())
        } else {
            ha1_base
        };
        let ha2 = alg.hash_hex(format!("{}:{}", method, uri).as_bytes());

        let nc_hex = format!("{:08x}", nc);

        let response = if !qop.is_empty() {
            alg.hash_hex(
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    ha1, self.nonce, nc_hex, cnonce, qop, ha2
                )
                .as_bytes(),
            )
        } else {
            alg.hash_hex(format!("{}:{}:{}", ha1, self.nonce, ha2).as_bytes())
        };

        let mut out = format!(
            "{SCHEME}username={u}, realm={r}, nonce={n}, uri={uri}, algorithm={alg}, response={resp}",
            u = quote(&auth.username),
            r = quote(&self.realm),
            n = quote(&self.nonce),
            uri = quote(uri),
            alg = alg.wire_name(),
            resp = quote(&response),
        );
        if !qop.is_empty() {
            out.push_str(&format!(
                ", qop={qop}, nc={nc_hex}, cnonce={}",
                quote(cnonce)
            ));
        }
        if let Some(opaque) = &self.opaque {
            out.push_str(&format!(", opaque={}", quote(opaque)));
        }
        Some(out)
    }

    pub(crate) fn covers(&self, url: &url::Url) -> bool {
        self.domain.is_empty()
            || self.domain.iter().any(|space| match url.join(space) {
                Ok(space) => Origin::same(&space, url) && url.path().starts_with(space.path()),
                Err(_) => false,
            })
    }
}

pub(crate) fn pick_supported_qop(qop: &str) -> Option<&'static str> {
    for token in qop.split(',') {
        if token.trim().eq_ignore_ascii_case("auth") {
            return Some("auth");
        }
    }
    None
}

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
    let mut guard = crate::util::lock(nonce_cache());
    if let Some(existing) = guard.get(nonce)
        && *existing >= u32::MAX - 1
    {
        guard.pop(nonce);
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

#[cfg(test)]
mod tests;
