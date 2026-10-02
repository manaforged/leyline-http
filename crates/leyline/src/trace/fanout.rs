use std::sync::Arc;

use super::{BodyEnd, Connect, Dns, Done, Head, Sent, Summary, Tls, Trace};

#[derive(Clone, Default)]
pub struct Fanout {
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, hook: impl Trace) -> Self {
        self.hooks.push(Arc::new(hook));
        self
    }

    fn each(&self, emit: impl Fn(&dyn Trace)) {
        for hook in &self.hooks {
            emit(hook.as_ref());
        }
    }
}

impl Trace for Fanout {
    fn dns(&self, ev: &Dns<'_>) {
        self.each(|hook| hook.dns(ev));
    }

    fn connect(&self, ev: &Connect<'_>) {
        self.each(|hook| hook.connect(ev));
    }

    fn tls(&self, ev: &Tls<'_>) {
        self.each(|hook| hook.tls(ev));
    }

    fn sent(&self, ev: &Sent<'_>) {
        self.each(|hook| hook.sent(ev));
    }

    fn head(&self, ev: &Head<'_>) {
        self.each(|hook| hook.head(ev));
    }

    fn done(&self, ev: &Done<'_>) {
        self.each(|hook| hook.done(ev));
    }

    fn summary(&self, ev: &Summary<'_>) {
        self.each(|hook| hook.summary(ev));
    }

    fn body(&self, ev: &BodyEnd<'_>) {
        self.each(|hook| hook.body(ev));
    }
}
