use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};

use leyline_bssl::ex_data::Index;
use leyline_bssl::ssl::{Ssl, SslConnectorBuilder, SslRef, SslSession, SslSessionCacheMode};
use lru::LruCache;

use crate::tls::error::TlsError;
use crate::util::lock;

const CAPACITY: NonZeroUsize = match NonZeroUsize::new(256) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

#[derive(Clone)]
pub(crate) struct SessionCache(Arc<Mutex<LruCache<String, Vec<u8>>>>);

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
        lock(&slot.cache.0).put(slot.key.clone(), der);
    }
}

impl SessionCache {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(LruCache::new(CAPACITY))))
    }

    pub(crate) fn key(host: &str, port: u16, route: Option<&url::Url>) -> String {
        let route = route.map(url::Url::as_str).unwrap_or_default();
        format!("{host}:{port}|{route}")
    }

    pub(crate) fn register(builder: &mut SslConnectorBuilder) -> Result<(), TlsError> {
        let index = slot_index()?;
        builder
            .set_session_cache_mode(SslSessionCacheMode::CLIENT | SslSessionCacheMode::NO_INTERNAL);
        builder.set_new_session_callback(move |ssl, session| store(ssl, session, index));
        Ok(())
    }

    pub(crate) fn attach(&self, ssl: &mut Ssl, key: &str) -> Result<(), TlsError> {
        let index = slot_index()?;
        let der = lock(&self.0).get(key).cloned();
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
                key: key.to_owned(),
            },
        );
        Ok(())
    }
}
