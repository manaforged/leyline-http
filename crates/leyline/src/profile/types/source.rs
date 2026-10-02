use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ProfileSource(Arc<str>);

impl ProfileSource {
    pub(crate) fn new(text: &str) -> Self {
        Self(text.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ProfileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProfileSource({} bytes)", self.0.len())
    }
}
