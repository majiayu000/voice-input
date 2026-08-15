use super::{AudioCapture, AudioDeviceReport, HotkeyEvent};
use crate::config::{AudioConfig, InsertionMode};
use crate::domain::{AudioChunk, FeedbackCue, SessionId};
use crate::ports::{Feedback, TextInjector};
use crate::runtime::RuntimeCommand;
use crate::service::{ServiceError, ServiceManager, ServicePaths, ServiceStatus};
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::mpsc;

const MESSAGE: &str = "voice-input currently supports macOS only";

pub struct MacSystemFeedback;

impl MacSystemFeedback {
    pub fn new(_enabled: bool) -> anyhow::Result<Self> {
        Ok(Self)
    }
}

impl Feedback for MacSystemFeedback {
    fn emit(&mut self, _cue: FeedbackCue) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct MacAudioCapture;

impl MacAudioCapture {
    pub fn new() -> Self {
        Self
    }
}

impl AudioCapture for MacAudioCapture {
    fn prepare(
        &mut self,
        _config: &AudioConfig,
        _sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        anyhow::bail!(MESSAGE)
    }

    fn start(
        &mut self,
        _session_id: SessionId,
        _config: &AudioConfig,
        _sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        anyhow::bail!(MESSAGE)
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct MacClipboardInjector;

impl MacClipboardInjector {
    pub fn new(_restore_after: Duration, _mode: InsertionMode) -> Self {
        Self
    }
}

#[async_trait]
impl TextInjector for MacClipboardInjector {
    async fn insert(&mut self, _text: &str) -> anyhow::Result<()> {
        anyhow::bail!(MESSAGE)
    }
}

pub struct MacHotkeyGuard;

pub fn accessibility_is_trusted(_prompt: bool) -> bool {
    false
}

pub fn install_hotkey(
    _specification: &str,
) -> anyhow::Result<(MacHotkeyGuard, mpsc::Receiver<HotkeyEvent>)> {
    anyhow::bail!(MESSAGE)
}

pub fn probe_audio_device(_sample_rate: u32) -> anyhow::Result<AudioDeviceReport> {
    anyhow::bail!(MESSAGE)
}

#[derive(Clone, Default)]
pub struct MainEventLoopHandle;

impl MainEventLoopHandle {
    pub fn new() -> Self {
        Self
    }

    pub fn request_exit(&self) {}
}

pub fn run_main_event_loop(_handle: &MainEventLoopHandle) {}

pub struct ShutdownSignalGuard;

pub fn install_shutdown_handler(
    _sender: mpsc::Sender<RuntimeCommand>,
    _event_loop: MainEventLoopHandle,
) -> anyhow::Result<ShutdownSignalGuard> {
    anyhow::bail!(MESSAGE)
}

pub struct PlatformServiceManager;

impl PlatformServiceManager {
    pub fn new(_paths: ServicePaths) -> Self {
        Self
    }
}

impl ServiceManager for PlatformServiceManager {
    fn install(&self, _executable: &std::path::Path) -> Result<ServiceStatus, ServiceError> {
        Err(ServiceError::Unsupported(MESSAGE))
    }

    fn start(&self) -> Result<ServiceStatus, ServiceError> {
        Err(ServiceError::Unsupported(MESSAGE))
    }

    fn stop(&self) -> Result<ServiceStatus, ServiceError> {
        Err(ServiceError::Unsupported(MESSAGE))
    }

    fn status(&self) -> Result<ServiceStatus, ServiceError> {
        Err(ServiceError::Unsupported(MESSAGE))
    }

    fn uninstall(&self) -> Result<ServiceStatus, ServiceError> {
        Err(ServiceError::Unsupported(MESSAGE))
    }
}
