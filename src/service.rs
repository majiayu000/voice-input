use crate::status::RuntimeSnapshot;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const SERVICE_LABEL: &str = "com.starlight.voiceinput";
pub const RUNTIME_APP_BUNDLE_NAME: &str = "Voice Input Runtime.app";
pub const LEGACY_RUNTIME_APP_BUNDLE_NAME: &str = "Voice Input.app";

#[derive(Debug, Clone, Serialize)]
pub struct ServicePaths {
    pub data_dir: PathBuf,
    pub app_bundle: PathBuf,
    pub binary: PathBuf,
    pub config: PathBuf,
    pub launch_agent: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub runtime_status: PathBuf,
    pub instance_lock: PathBuf,
    pub latency_history: PathBuf,
}

impl ServicePaths {
    pub fn discover(config: Option<&Path>) -> Result<Self, ServiceError> {
        let home = dirs::home_dir().ok_or(ServiceError::HomeDirectoryUnavailable)?;
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| home.join("Library/Application Support"))
            .join("voice-input");
        let logs = home.join("Library/Logs/voice-input");
        let app_bundle = data_dir.join(RUNTIME_APP_BUNDLE_NAME);
        Ok(Self {
            binary: app_bundle.join("Contents/MacOS/voice-input"),
            app_bundle,
            config: config
                .map(Path::to_path_buf)
                .unwrap_or_else(crate::config::VoiceConfig::default_path),
            launch_agent: home
                .join("Library/LaunchAgents")
                .join(format!("{SERVICE_LABEL}.plist")),
            stdout_log: logs.join("stdout.log"),
            stderr_log: logs.join("stderr.log"),
            runtime_status: data_dir.join("runtime/status.json"),
            instance_lock: data_dir.join("runtime/daemon.lock"),
            latency_history: data_dir.join("metrics/latency.jsonl"),
            data_dir,
        })
    }

    #[cfg(test)]
    pub(crate) fn in_root(root: &Path) -> Self {
        let data_dir = root.join("data");
        let app_bundle = data_dir.join(RUNTIME_APP_BUNDLE_NAME);
        Self {
            binary: app_bundle.join("Contents/MacOS/voice-input"),
            app_bundle,
            config: root.join("config/config.toml"),
            launch_agent: root.join(format!("{SERVICE_LABEL}.plist")),
            stdout_log: root.join("logs/stdout.log"),
            stderr_log: root.join("logs/stderr.log"),
            runtime_status: data_dir.join("runtime/status.json"),
            instance_lock: data_dir.join("runtime/daemon.lock"),
            latency_history: data_dir.join("metrics/latency.jsonl"),
            data_dir,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatus {
    pub label: &'static str,
    pub installed: bool,
    pub loaded: bool,
    pub launchd_state: Option<String>,
    pub pid: Option<u32>,
    pub last_exit_code: Option<i32>,
    pub runtime: Option<RuntimeSnapshot>,
    pub paths: ServicePaths,
}

pub trait ServiceManager {
    fn install(&self, executable: &Path) -> Result<ServiceStatus, ServiceError>;
    fn start(&self) -> Result<ServiceStatus, ServiceError>;
    fn stop(&self) -> Result<ServiceStatus, ServiceError>;
    fn status(&self) -> Result<ServiceStatus, ServiceError>;
    fn uninstall(&self) -> Result<ServiceStatus, ServiceError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("the user home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("voice-input is not installed; run `voice-input install` first")]
    NotInstalled,
    #[error("service management is unsupported: {0}")]
    Unsupported(&'static str),
    #[error("service IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("launchctl {action} failed: {detail}")]
    Launchctl {
        action: &'static str,
        detail: String,
    },
    #[error("service packaging failed: {0}")]
    Packaging(String),
    #[error("runtime status is invalid: {0}")]
    InvalidRuntimeStatus(#[from] serde_json::Error),
}
