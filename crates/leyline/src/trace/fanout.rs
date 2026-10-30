use std::sync::Arc;

use super::{Event, Trace};

#[derive(Clone)]
pub(crate) struct Fanout {
    hooks: Vec<Arc<dyn Trace>>,
}

impl std::fmt::Debug for Fanout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fanout")
            .field("hooks", &self.hooks.len())
            .finish()
    }
}

impl Fanout {
    pub(crate) fn new(hooks: Vec<Arc<dyn Trace>>) -> Self {
        Self { hooks }
    }
}

impl Trace for Fanout {
    fn event(&self, event: &Event<'_>) {
        for hook in &self.hooks {
            hook.event(event);
        }
    }
}
