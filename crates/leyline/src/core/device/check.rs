use crate::core::error::Result;
use crate::core::{Session, SessionIdentity};

use super::secret::ProxyKey;
use super::{Device, config};

impl Device {
    pub fn check(&self, session: &Session) -> Result<()> {
        let id = session.identity();
        let mut differences = match self.profile_id.as_deref() {
            Some(expected) => profile_difference(expected, &id),
            None => self.identity_differences(&id),
        };
        let profile_changed = !differences.is_empty();
        differences.extend(self.setting_differences(session, &id));
        differences.extend(self.strict_gaps());
        if differences.is_empty() {
            return Ok(());
        }
        let err = config(format!(
            "the session differs from the device: {}",
            differences.join("; ")
        ));
        Err(if profile_changed {
            err.with_source(crate::core::error::ProfileChanged)
        } else {
            err
        })
    }

    fn identity_differences(&self, id: &SessionIdentity) -> Vec<String> {
        let mut differences = Vec::new();
        if self.identity != id.to_identity() {
            differences.push(format!(
                "identity {:?} (device {:?})",
                id.to_identity(),
                self.identity
            ));
        }
        if self.platform != id.platform() {
            differences.push(format!(
                "platform {:?} (device {:?})",
                id.platform(),
                self.platform
            ));
        }
        if self.brand != id.brand() {
            differences.push(format!("brand {:?} (device {:?})", id.brand(), self.brand));
        }
        if let Some(expected) = self.user_agent.as_deref()
            && expected != id.user_agent()
        {
            differences.push(format!(
                "user_agent {:?} (device {expected:?})",
                id.user_agent()
            ));
        }
        differences
    }

    fn strict_gaps(&self) -> Vec<String> {
        let mut gaps = Vec::new();
        if !self.strict {
            return gaps;
        }
        if self.profile_id.is_none() {
            gaps.push("strict device has no profile_id".to_owned());
        }
        if self.proxy.is_none() && !self.env_proxy {
            gaps.push("strict device has no proxy and env_proxy is false".to_owned());
        }
        gaps
    }

    fn setting_differences(&self, session: &Session, id: &SessionIdentity) -> Vec<String> {
        let mut differences = Vec::new();
        if self.languages.as_deref() != id.languages() {
            differences.push(format!(
                "languages {:?} (device {:?})",
                id.languages(),
                self.languages
            ));
        }
        let proxy = session.proxy_url();
        if !self.env_proxy
            && self.proxy.as_ref().map(ProxyKey::of) != proxy.as_ref().map(ProxyKey::of)
        {
            differences.push(format!("proxy {proxy:?} (device {:?})", self.proxy));
        }
        differences
    }
}

fn profile_difference(expected: &str, id: &SessionIdentity) -> Vec<String> {
    match id.profile_id() {
        Some(actual) if actual == expected => Vec::new(),
        actual => vec![format!("profile_id {actual:?} (device {expected:?})")],
    }
}
