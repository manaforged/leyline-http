use std::sync::{Mutex, PoisonError, Weak};

use crate::cookie::Jar;
use crate::core::Result;
use crate::core::session::{Identity, Session, SessionInner};

struct Derived {
    base: Weak<SessionInner>,
    index: usize,
    session: Session,
}

#[derive(Default)]
pub(super) struct IdentitySessions {
    derived: Mutex<Vec<Derived>>,
}

impl std::fmt::Debug for IdentitySessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentitySessions")
            .field("sessions", &self.lock().len())
            .finish()
    }
}

impl IdentitySessions {
    pub(super) fn session(
        &self,
        base: &Session,
        index: usize,
        identity: Identity,
    ) -> Result<Session> {
        if let Some(session) = self.find(base, index) {
            return Ok(session);
        }
        let session = base
            .with_identity(identity)?
            .without_proxy_pool()
            .with_cookie_jar(Jar::new());
        Ok(self.insert(base, index, session))
    }

    fn find(&self, base: &Session, index: usize) -> Option<Session> {
        let mut derived = self.lock();
        derived.retain(|entry| entry.base.strong_count() > 0);
        derived
            .iter()
            .find(|entry| entry.index == index && base.is_inner(&entry.base))
            .map(|entry| entry.session.clone())
    }

    fn insert(&self, base: &Session, index: usize, session: Session) -> Session {
        let mut derived = self.lock();
        if let Some(existing) = derived
            .iter()
            .find(|entry| entry.index == index && base.is_inner(&entry.base))
        {
            return existing.session.clone();
        }
        derived.push(Derived {
            base: base.downgrade(),
            index,
            session: session.clone(),
        });
        session
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Derived>> {
        self.derived.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
