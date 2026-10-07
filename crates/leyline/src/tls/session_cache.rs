use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};

use leyline_bssl::ex_data::Index;
use leyline_bssl::ssl::{Ssl, SslConnectorBuilder, SslRef, SslSession, SslSessionCacheMode};
use lru::LruCache;
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;
use crate::tls::trust::TrustIdentity;
use crate::util::lock;

const CAPACITY: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

const POLICY_SEPARATOR: &str = "#trust=";
const CREDENTIAL_SEPARATOR: &str = "#proxy-credentials=";

#[derive(Clone)]
pub(crate) struct SessionCache {
    entries: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    policy: TrustIdentity,
}

struct Slot {
    cache: SessionCache,
    key: String,
}

fn slot_index() -> Result<Index<Ssl, Slot>, TlsError> {
    static SLOT: OnceLock<Index<Ssl, Slot>> = OnceLock::new();
    if let Some(index) = SLOT.get() {
        return Ok(*index);
    }
    let index = Ssl::new_ex_index().map_err(TlsError::from_stack)?;
    Ok(*SLOT.get_or_init(|| index))
}

fn store(ssl: &mut SslRef, session: SslSession, index: Index<Ssl, Slot>) {
    if let Some(slot) = ssl.ex_data(index)
        && let Ok(der) = session.to_der()
    {
        lock(&slot.cache.entries).put(slot.key.clone(), der);
    }
}

fn fresh(der: &[u8], now_secs: u64) -> bool {
    SslSession::from_der(der)
        .is_ok_and(|session| session.time().saturating_add(u64::from(session.timeout())) > now_secs)
}

impl SessionCache {
    pub(crate) fn new(policy: TrustIdentity) -> Self {
        Self {
            entries: Arc::new(Mutex::new(LruCache::new(CAPACITY))),
            policy,
        }
    }

    pub(crate) fn cleared(&self) -> Self {
        Self::new(self.policy.clone())
    }

    fn bound(&self, key: &str) -> String {
        format!("{key}{POLICY_SEPARATOR}{}", self.policy.as_str())
    }

    pub(crate) fn key(host: &str, port: u16, route: Option<&url::Url>) -> String {
        let Some(route) = route else {
            return format!("{host}:{port}|");
        };
        let mut bare = route.clone();
        let credentials = format!("{}:{}", route.username(), route.password().unwrap_or(""));
        let stripped = bare.set_username("").is_ok() && bare.set_password(None).is_ok();
        if !stripped || credentials == ":" {
            return format!("{host}:{port}|{bare}");
        }
        let digest = hex::encode(Sha256::digest(credentials.as_bytes()));
        format!("{host}:{port}|{bare}{CREDENTIAL_SEPARATOR}{digest}")
    }

    pub(crate) fn register(builder: &mut SslConnectorBuilder) -> Result<(), TlsError> {
        let index = slot_index()?;
        builder
            .set_session_cache_mode(SslSessionCacheMode::CLIENT | SslSessionCacheMode::NO_INTERNAL);
        builder.set_new_session_callback(move |ssl, session| store(ssl, session, index));
        Ok(())
    }

    pub(crate) fn export(&self, now_secs: u64) -> Vec<(String, Vec<u8>)> {
        let cache = lock(&self.entries);
        let mut entries: Vec<(String, Vec<u8>)> = cache
            .iter()
            .filter(|(_, der)| fresh(der, now_secs))
            .map(|(key, der)| (key.clone(), der.clone()))
            .collect();
        entries.reverse();
        entries
    }

    pub(crate) fn import(&self, entries: &[(String, Vec<u8>)], now_secs: u64) {
        let suffix = format!("{POLICY_SEPARATOR}{}", self.policy.as_str());
        let mut cache = lock(&self.entries);
        for (key, der) in entries
            .iter()
            .filter(|(key, der)| key.ends_with(&suffix) && fresh(der, now_secs))
        {
            cache.put(key.clone(), der.clone());
        }
    }

    pub(crate) fn attach(&self, ssl: &mut Ssl, key: &str) -> Result<(), TlsError> {
        let index = slot_index()?;
        let key = self.bound(key);
        let der = lock(&self.entries).get(&key).cloned();
        if let Some(der) = der
            && let Ok(session) = SslSession::from_der(&der)
        {
            // SAFETY: BoringSSL requires `set_session` to be called on an Ssl not yet handed to `connect()`. `ssl` was just constructed via `config.into_ssl` and has not started its handshake. The `SslSession` is owned for the duration of this block. No concurrent access.
            drop(unsafe { ssl.set_session(&session) });
        }
        ssl.set_ex_data(
            index,
            Slot {
                cache: self.clone(),
                key,
            },
        );
        Ok(())
    }
}
