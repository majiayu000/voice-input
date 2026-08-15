use crate::config::AudioConfig;
use crate::domain::{AudioChunk, SessionId};
use serde::Serialize;
use std::path::Path;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize)]
pub struct AudioDeviceReport {
    pub name: String,
    pub input_sample_rate: u32,
    pub output_sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    pub native_16khz: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
mod launchd;

#[cfg(target_os = "macos")]
pub use macos::{
    accessibility_is_trusted, install_hotkey, install_shutdown_handler, probe_audio_device,
    run_main_event_loop, MacAudioCapture, MacClipboardInjector, MacHotkeyGuard, MacSystemFeedback,
    MainEventLoopHandle, ShutdownSignalGuard,
};

#[cfg(target_os = "macos")]
pub use launchd::LaunchdServiceManager as PlatformServiceManager;

#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    accessibility_is_trusted, install_hotkey, install_shutdown_handler, probe_audio_device,
    run_main_event_loop, MacAudioCapture, MacClipboardInjector, MacHotkeyGuard, MacSystemFeedback,
    MainEventLoopHandle, PlatformServiceManager, ShutdownSignalGuard,
};

pub trait AudioCapture {
    fn prepare(
        &mut self,
        _config: &AudioConfig,
        _sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        anyhow::bail!("audio capture does not support prewarming")
    }

    fn start(
        &mut self,
        session_id: SessionId,
        config: &AudioConfig,
        sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport>;

    fn stop(&mut self) -> anyhow::Result<()>;
}

pub async fn record_wav(
    capture: &mut impl AudioCapture,
    config: &AudioConfig,
    duration: std::time::Duration,
    output: &Path,
) -> anyhow::Result<AudioDeviceReport> {
    let (sender, mut receiver) = mpsc::channel(config.channel_capacity);
    let session_id = SessionId::new();
    let report = capture.start(session_id, config, sender)?;
    let deadline = tokio::time::Instant::now() + duration;
    let mut samples =
        Vec::with_capacity((config.sample_rate as f64 * duration.as_secs_f64()) as usize);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            chunk = receiver.recv() => {
                match chunk {
                    Some(chunk) => samples.extend_from_slice(&chunk.samples),
                    None => break,
                }
            }
        }
    }
    capture.stop()?;
    while let Ok(chunk) = receiver.try_recv() {
        samples.extend_from_slice(&chunk.samples);
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: config.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output, spec)?;
    for sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(report)
}
