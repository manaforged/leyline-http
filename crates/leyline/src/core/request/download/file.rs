use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::saved::FileVersion;
use crate::core::{Error, Kind, Result};
use crate::util::{atomic, lock};

const CHECKPOINT_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 16 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub(super) version: FileVersion,
    pub(super) request: [u8; 32],
    pub(super) url: String,
    pub(super) etag: Option<String>,
    pub(super) total: Option<u64>,
    pub(super) offset: u64,
}

pub(super) struct Partial {
    pub(super) target: PathBuf,
    data_path: PathBuf,
    metadata_path: PathBuf,
    _lock: File,
    data: Mutex<Data>,
}

struct Data {
    file: File,
    metadata: Option<Metadata>,
    written: u64,
}

impl Partial {
    pub(super) fn open(target: PathBuf, request: [u8; 32], resume: bool) -> Result<Self> {
        let data_path = sidecar(&target, ".leyline.part")?;
        let metadata_path = sidecar(&target, ".leyline.part.json")?;
        let locked = open(&sidecar(&target, ".leyline.lock")?)?;
        locked.try_lock().map_err(|error| {
            Error::new(Kind::Io)
                .with_message("download target is locked")
                .with_source(error)
        })?;
        let mut file = open(&data_path)?;
        let metadata = if resume {
            read_metadata(&metadata_path)?
        } else {
            None
        };
        let actual_length = file.metadata()?.len();
        let metadata = metadata.filter(|meta| {
            meta.request == request
                && meta.etag.as_deref().is_some_and(super::strong_etag)
                && meta.total.is_none_or(|total| meta.offset <= total)
                && meta.offset <= actual_length
        });
        let written = metadata.as_ref().map_or(0, |meta| meta.offset);
        file.set_len(written)?;
        file.seek(SeekFrom::Start(written))?;
        Ok(Self {
            target,
            data_path,
            metadata_path,
            _lock: locked,
            data: Mutex::new(Data {
                file,
                metadata,
                written,
            }),
        })
    }

    pub(super) fn metadata(&self) -> Option<Metadata> {
        lock(&self.data).metadata.clone()
    }

    pub(super) fn prefix(&self, checksum: bool) -> Result<Sha256> {
        let mut data = lock(&self.data);
        let mut digest = Sha256::new();
        if checksum {
            data.file.rewind()?;
            let mut bytes = [0u8; 64 * 1024];
            loop {
                let n = data.file.read(&mut bytes)?;
                if n == 0 {
                    break;
                }
                digest.update(&bytes[..n]);
            }
            let written = data.written;
            data.file.seek(SeekFrom::Start(written))?;
        }
        Ok(digest)
    }

    pub(super) fn begin(&self, mut metadata: Metadata, append: bool) -> Result<()> {
        let mut data = lock(&self.data);
        if !append {
            data.file.set_len(0)?;
            data.file.rewind()?;
            data.written = 0;
        }
        metadata.offset = data.written;
        data.metadata = Some(metadata);
        self.checkpoint_locked(&mut data)
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut data = lock(&self.data);
        data.file.write_all(bytes)?;
        data.written += bytes.len() as u64;
        let saved = data.metadata.as_ref().map_or(0, |metadata| metadata.offset);
        if data.written - saved >= CHECKPOINT_BYTES {
            self.checkpoint_locked(&mut data)?;
        }
        Ok(())
    }

    pub(super) fn checkpoint(&self) -> Result<()> {
        self.checkpoint_locked(&mut lock(&self.data))
    }

    fn checkpoint_locked(&self, data: &mut Data) -> Result<()> {
        data.file.sync_all()?;
        if let Some(metadata) = &mut data.metadata {
            metadata.offset = data.written;
            let bytes = serde_json::to_vec(metadata).map_err(Error::from_json)?;
            atomic::write_atomic(&self.metadata_path, &bytes, atomic::FileMode::Private)?;
        }
        Ok(())
    }

    pub(super) fn discard(&self) -> Result<()> {
        let mut data = lock(&self.data);
        data.file.set_len(0)?;
        data.file.rewind()?;
        data.written = 0;
        self.checkpoint_locked(&mut data)
    }

    pub(super) fn commit(&self, waiting: &AtomicBool) -> Result<()> {
        let mut data = lock(&self.data);
        self.checkpoint_locked(&mut data)?;
        if !waiting.load(Ordering::Acquire) {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into());
        }
        atomic::commit(&self.data_path, &self.target)?;
        match std::fs::remove_file(&self.metadata_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::from(error)
                .with_message("download committed but removing its metadata failed")),
        }
    }
}

fn sidecar(path: &Path, suffix: &str) -> Result<PathBuf> {
    let mut name = path
        .file_name()
        .ok_or_else(|| Error::config("download path has no file name"))?
        .to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

fn open(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn read_metadata(path: &Path) -> Result<Option<Metadata>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(
            Error::new(Kind::BodyLimit).with_message("download metadata exceeds size limit")
        );
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(Error::from_json)
}
