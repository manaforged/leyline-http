use super::{AUTHORIZATION, SessionBuilder};

#[derive(Clone)]
pub(crate) struct BearerToken {
    pub(crate) value: String,
    pub(crate) slot: usize,
}

impl SessionBuilder {
    pub fn bearer_auth(mut self, token: &str) -> Self {
        let value = format!("Bearer {token}");
        self.check_header(AUTHORIZATION, &value);
        self.remove_default(AUTHORIZATION);
        self.bearer = Some(BearerToken {
            value,
            slot: self.default_headers.len(),
        });
        self
    }

    pub(super) fn remove_default(&mut self, name: &str) {
        if let Some(bearer) = self.bearer.as_mut() {
            let removed = self.default_headers[..bearer.slot]
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(name))
                .count();
            bearer.slot -= removed;
        }
        self.default_headers
            .retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }

    pub(super) fn push_default(&mut self, name: String, value: String) {
        match self
            .bearer
            .take_if(|_| name.eq_ignore_ascii_case(AUTHORIZATION))
        {
            Some(bearer) => self.default_headers.insert(bearer.slot, (name, value)),
            None => self.default_headers.push((name, value)),
        }
    }
}
