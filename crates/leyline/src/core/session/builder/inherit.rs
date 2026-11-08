use std::sync::Arc;

use super::super::Session;
use super::SessionBuilder;
use crate::error::{Error, Result};
use crate::profile::TcpProfile;
use crate::tls::FingerprintConnector;

#[derive(Clone)]
pub(super) struct Parent {
    session: Session,
    pub(super) separate_pool: bool,
    pub(super) own_limits: bool,
}

impl Session {
    /// Returns a builder with this session's settings and cookie jar that shares its connection pool by default.
    pub fn to_builder(&self) -> SessionBuilder {
        let mut builder = self.inner.inputs.clone();
        builder.parent = Some(Parent {
            session: self.clone(),
            separate_pool: false,
            own_limits: false,
        });
        builder
    }
}

impl Parent {
    pub(super) fn shared_connector(&self, child: &SessionBuilder) -> Option<FingerprintConnector> {
        let parent = &self.session.inner;
        let same_inputs = child.pool_inputs() == parent.inputs.pool_inputs();
        (!self.separate_pool && same_inputs).then(|| parent.connector.clone())
    }

    pub(super) fn share_with(self, child: &mut Session, shared: Option<TcpProfile>) -> Result<()> {
        let parent = &self.session.inner;
        let inner = Arc::make_mut(&mut child.inner);
        inner.shutdown = parent.shutdown.clone();
        inner.writers = Arc::clone(&parent.writers);
        if !self.own_limits {
            inner.host_limits = parent.host_limits.clone();
        }
        let Some(tcp) = shared else {
            return Ok(());
        };
        inner.hsts = Arc::clone(&parent.hsts);
        let same_identity = inner.profile_hash() == parent.profile_hash();
        if !same_identity || parent.connector.dialer().tcp_profile() != &tcp {
            inner.connector = parent
                .connector
                .with_profile(&inner.profile, tcp)
                .map_err(Error::from)?;
        }
        inner.pool = if same_identity {
            Arc::clone(&parent.pool)
        } else {
            Arc::new(parent.pool.shared_view(inner.profile_hash()))
        };
        Ok(())
    }
}
