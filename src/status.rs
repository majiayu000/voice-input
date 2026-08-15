use crate::domain::LatencyReport;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const RUNTIME_STATUS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase {
    Starting,
    Ready,
    Capturing,
    Finalizing,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub schema_version: u32,
    pub pid: u32,
    pub phase: RuntimePhase,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub hotkey: String,
    pub active_session: Option<String>,
    pub sessions_completed: u64,
    pub last_latency: Option<LatencyReport>,
    pub last_error: Option<String>,
}

impl RuntimeSnapshot {
    pub fn starting(hotkey: impl Into<String>) -> Self {
        let now = unix_time_ms();
        Self {
            schema_version: RUNTIME_STATUS_SCHEMA_VERSION,
            pid: std::process::id(),
            phase: RuntimePhase::Starting,
            started_at_ms: now,
            updated_at_ms: now,
            hotkey: hotkey.into(),
            active_session: None,
            sessions_completed: 0,
            last_latency: None,
            last_error: None,
        }
    }

    pub fn transition(&mut self, phase: RuntimePhase) {
        self.phase = phase;
        self.updated_at_ms = unix_time_ms();
    }

    pub fn record_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
        self.updated_at_ms = unix_time_ms();
    }
}

pub trait StatusPublisher: Send {
    fn publish(&mut self, snapshot: &RuntimeSnapshot) -> anyhow::Result<()>;
}

impl<T> StatusPublisher for Box<T>
where
    T: StatusPublisher + ?Sized,
{
    fn publish(&mut self, snapshot: &RuntimeSnapshot) -> anyhow::Result<()> {
        (**self).publish(snapshot)
    }
}

#[derive(Debug)]
pub struct FileStatusPublisher {
    shared: std::sync::Arc<(std::sync::Mutex<PublisherState>, std::sync::Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Default)]
struct PublisherState {
    latest: Option<RuntimeSnapshot>,
    closed: bool,
}

impl FileStatusPublisher {
    pub fn new(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let shared = std::sync::Arc::new((
            std::sync::Mutex::new(PublisherState::default()),
            std::sync::Condvar::new(),
        ));
        let worker_shared = std::sync::Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("voice-input-status".to_owned())
            .spawn(move || loop {
                let snapshot = {
                    let (lock, ready) = &*worker_shared;
                    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    while state.latest.is_none() && !state.closed {
                        state = ready
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                    match state.latest.take() {
                        Some(snapshot) => snapshot,
                        None if state.closed => break,
                        None => continue,
                    }
                };
                if let Err(error) = write_json_atomically(&path, &snapshot) {
                    tracing::warn!(%error, "failed to write runtime status");
                }
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
}

impl StatusPublisher for FileStatusPublisher {
    fn publish(&mut self, snapshot: &RuntimeSnapshot) -> anyhow::Result<()> {
        let (lock, ready) = &*self.shared;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime status publisher lock is poisoned"))?;
        anyhow::ensure!(!state.closed, "runtime status publisher is closed");
        state.latest = Some(snapshot.clone());
        ready.notify_one();
        Ok(())
    }
}

impl Drop for FileStatusPublisher {
    fn drop(&mut self) {
        let (lock, ready) = &*self.shared;
        {
            let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.closed = true;
            ready.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("runtime status worker panicked");
            }
        }
    }
}

pub fn read_runtime_snapshot(path: &Path) -> anyhow::Result<Option<RuntimeSnapshot>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub struct InstanceLease {
    file: File,
    path: PathBuf,
}

impl InstanceLease {
    pub fn acquire(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.try_lock_exclusive().map_err(|error| {
            anyhow::anyhow!(
                "another voice-input daemon already owns {}: {error}",
                path.display()
            )
        })?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLease {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.file) {
            tracing::warn!(path = %self.path.display(), %error, "failed to release instance lease");
        }
    }
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("status.json");
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_status_is_atomic_and_readable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("status.json");
        let mut publisher = FileStatusPublisher::new(&path).unwrap();
        let mut snapshot = RuntimeSnapshot::starting("control+shift+space");
        snapshot.transition(RuntimePhase::Ready);

        publisher.publish(&snapshot).unwrap();
        drop(publisher);
        let decoded = read_runtime_snapshot(&path).unwrap().unwrap();

        assert_eq!(decoded.phase, RuntimePhase::Ready);
        assert_eq!(decoded.schema_version, RUNTIME_STATUS_SCHEMA_VERSION);
        assert!(directory.path().read_dir().unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

    #[test]
    fn exclusive_lease_rejects_second_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("daemon.lock");
        let first = InstanceLease::acquire(&path).unwrap();

        let error = InstanceLease::acquire(&path).err().unwrap();
        assert!(error.to_string().contains("already owns"));

        drop(first);
        InstanceLease::acquire(path).unwrap();
    }
}
