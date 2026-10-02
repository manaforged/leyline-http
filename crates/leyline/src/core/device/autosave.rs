use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::core::error::Result;
use crate::core::{Session, Tab};
use crate::util::autosave::{Autosave, SaveJob, SaveTarget, Schedule};
use crate::util::lock;

use super::Device;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeviceAutosaveOptions {
    pub interval: Duration,
    pub state_interval: Duration,
}

impl DeviceAutosaveOptions {
    #[must_use]
    pub const fn new(interval: Duration) -> Self {
        Self {
            interval,
            state_interval: Duration::from_secs(300),
        }
    }

    #[must_use]
    pub const fn state_interval(mut self, state_interval: Duration) -> Self {
        self.state_interval = state_interval;
        self
    }
}

impl From<Duration> for DeviceAutosaveOptions {
    fn from(interval: Duration) -> Self {
        Self::new(interval)
    }
}

pub struct DeviceAutosave {
    path: Arc<Path>,
    device: Arc<Mutex<Device>>,
    tab: Arc<Mutex<Option<Tab>>>,
    autosave: Autosave,
}

struct DeviceTarget {
    device: Arc<Mutex<Device>>,
    tab: Arc<Mutex<Option<Tab>>>,
    session: Session,
    path: Arc<Path>,
    saved: Option<u64>,
}

impl Device {
    pub fn autosave(
        &self,
        session: &Session,
        path: impl Into<PathBuf>,
        options: impl Into<DeviceAutosaveOptions>,
    ) -> DeviceAutosave {
        let options = options.into();
        let path: Arc<Path> = Arc::from(path.into());
        let device = Arc::new(Mutex::new(self.clone()));
        let tab = Arc::new(Mutex::new(None));
        let target = DeviceTarget {
            device: Arc::clone(&device),
            tab: Arc::clone(&tab),
            session: session.clone(),
            path: Arc::clone(&path),
            saved: None,
        };
        let schedule = Schedule {
            interval: options.interval,
            periodic: Some(options.state_interval),
            label: "device",
        };
        DeviceAutosave {
            path,
            device,
            tab,
            autosave: Autosave::spawn(target, session.cookies().changes(), schedule),
        }
    }
}

impl DeviceAutosave {
    pub fn update(&self, change: impl FnOnce(&mut Device)) {
        change(&mut lock(&self.device));
        self.autosave.touch();
    }

    pub fn track(&self, tab: &Tab) {
        *lock(&self.tab) = Some(tab.clone());
        self.autosave.touch();
    }

    #[must_use]
    pub fn device(&self) -> Device {
        snapshot(&self.device, &self.tab)
    }

    pub async fn flush(&self) -> Result<()> {
        self.autosave.flush().await
    }

    pub async fn shutdown(self) -> Result<()> {
        self.autosave.shutdown().await
    }
}

impl std::fmt::Debug for DeviceAutosave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAutosave")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SaveTarget for DeviceTarget {
    fn prepare(&mut self) -> Option<SaveJob> {
        let jar = self.session.cookies().clone();
        let generation = *jar.changes().borrow();
        let jar_dirty = self.saved != Some(generation);
        let mut device = snapshot(&self.device, &self.tab);
        device.state = self.session.state();
        let jar_path = device.jar_path.clone();
        device.jar = jar_path.is_none().then(|| jar.clone());
        let path = Arc::clone(&self.path);
        Some(SaveJob {
            generation,
            write: Box::new(move || {
                if let Some(jar_path) = jar_path.filter(|_| jar_dirty) {
                    jar.save_to(jar_path)?;
                }
                device.save_to(&*path)
            }),
        })
    }

    fn saved(&mut self, generation: u64) {
        self.saved = Some(generation);
    }
}

fn snapshot(device: &Mutex<Device>, tab: &Mutex<Option<Tab>>) -> Device {
    let mut device = lock(device).clone();
    if let Some(tab) = lock(tab).as_ref() {
        device.page = tab.current();
    }
    device
}
