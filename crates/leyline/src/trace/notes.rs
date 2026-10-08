use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug, Default)]
pub(crate) struct Notes {
    pub(crate) tag: Option<String>,
    pub(crate) proxy: Option<String>,
    pub(crate) browser: Option<crate::profile::Browser>,
}

pub(crate) type SharedNotes = Arc<Mutex<Notes>>;

pub(crate) fn note_tag(tag: Option<String>) {
    if let Some(tag) = tag {
        edit(|notes| notes.tag = Some(tag));
    }
}

pub(crate) fn note_proxy(proxy: Option<&str>) {
    if !super::on() {
        return;
    }
    let proxy = proxy.map(crate::util::redact);
    edit(|notes| notes.proxy = proxy);
}

pub(crate) fn note_browser(browser: Option<crate::profile::Browser>) {
    if !super::on() {
        return;
    }
    edit(|notes| notes.browser = browser);
}

pub(crate) fn take(shared: &SharedNotes) -> Notes {
    std::mem::take(&mut *shared.lock().unwrap_or_else(PoisonError::into_inner))
}

fn edit(change: impl FnOnce(&mut Notes)) {
    super::with(|ctx| change(&mut ctx.notes.lock().unwrap_or_else(PoisonError::into_inner)));
}
