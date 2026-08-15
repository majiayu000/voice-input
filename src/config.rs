use crate::domain::ASR_SAMPLE_RATE;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub hotkey: String,
    pub audio: AudioConfig,
    pub asr: AsrConfig,
    pub refiner: RefinerConfig,
    pub insertion: InsertionConfig,
    pub feedback: FeedbackConfig,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            hotkey: "control+shift+space".to_owned(),
            audio: AudioConfig::default(),
            asr: AsrConfig::default(),
            refiner: RefinerConfig::default(),
            insertion: InsertionConfig::default(),
            feedback: FeedbackConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    pub backend: AsrBackend,
    pub model_path: Option<PathBuf>,
    pub language: String,
    pub initial_prompt: Option<String>,
    pub threads: usize,
    pub use_gpu: bool,
    pub flash_attention: bool,
    pub max_audio_seconds: u64,
    pub vad: VadConfig,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            backend: AsrBackend::Whisper,
            model_path: None,
            language: "auto".to_owned(),
            initial_prompt: None,
            threads: 4,
            use_gpu: true,
            flash_attention: true,
            max_audio_seconds: 120,
            vad: VadConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    #[default]
    Whisper,
    Scripted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VadConfig {
    pub enabled: bool,
    pub rms_threshold: f32,
    pub min_speech_ms: u32,
    pub speech_pad_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rms_threshold: 0.008,
            min_speech_ms: 80,
            speech_pad_ms: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RefinerConfig {
    pub backend: RefinerBackend,
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub timeout_ms: u64,
    pub max_output_tokens: u32,
    pub allow_remote: bool,
    pub failure_mode: RefinerFailureMode,
    pub system_prompt: String,
}

impl Default for RefinerConfig {
    fn default() -> Self {
        Self {
            backend: RefinerBackend::Identity,
            base_url: "http://127.0.0.1:11434/v1".to_owned(),
            model: String::new(),
            api_key_env: None,
            timeout_ms: 900,
            max_output_tokens: 512,
            allow_remote: false,
            failure_mode: RefinerFailureMode::Bypass,
            system_prompt: "Polish this speech transcript without changing its meaning. Fix punctuation, casing, and obvious recognition errors. Return only the corrected text.".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefinerBackend {
    #[default]
    Identity,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefinerFailureMode {
    #[default]
    Bypass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub chunk_ms: u32,
    pub channel_capacity: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: ASR_SAMPLE_RATE,
            chunk_ms: 80,
            channel_capacity: 32,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InsertionConfig {
    pub restore_clipboard_after_ms: u64,
    pub mode: InsertionMode,
}

impl Default for InsertionConfig {
    fn default() -> Self {
        Self {
            restore_clipboard_after_ms: 160,
            mode: InsertionMode::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertionMode {
    /// Prefer direct AXSelectedText insertion, then use the pasteboard.
    #[default]
    Auto,
    /// Require direct Accessibility insertion; never mutate the pasteboard.
    Accessibility,
    /// Always use Command-V with a complete pasteboard snapshot.
    Clipboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackConfig {
    pub audible: bool,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self { audible: true }
    }
}

impl VoiceConfig {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_path);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn write_default(path: Option<&Path>) -> anyhow::Result<PathBuf> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            std::fs::write(&path, toml::to_string_pretty(&Self::default())?)?;
        }
        Ok(path)
    }

    pub fn save(&self, path: Option<&Path>) -> anyhow::Result<PathBuf> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, toml::to_string_pretty(self)?)?;
        std::fs::rename(&temporary, &path)?;
        Ok(path)
    }

    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("voice-input")
            .join("config.toml")
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.hotkey.trim().is_empty(), "hotkey must not be empty");
        anyhow::ensure!(
            self.audio.sample_rate == ASR_SAMPLE_RATE,
            "the ASR contract currently requires a 16 kHz sample rate"
        );
        anyhow::ensure!(
            (20..=500).contains(&self.audio.chunk_ms),
            "audio.chunk_ms must be between 20 and 500"
        );
        anyhow::ensure!(
            (4..=256).contains(&self.audio.channel_capacity),
            "audio.channel_capacity must be between 4 and 256"
        );
        anyhow::ensure!(
            (1..=128).contains(&self.asr.threads),
            "asr.threads must be between 1 and 128"
        );
        anyhow::ensure!(
            (1..=3_600).contains(&self.asr.max_audio_seconds),
            "asr.max_audio_seconds must be between 1 and 3600"
        );
        anyhow::ensure!(
            self.asr.vad.rms_threshold.is_finite()
                && (0.0..=1.0).contains(&self.asr.vad.rms_threshold),
            "asr.vad.rms_threshold must be between 0 and 1"
        );
        anyhow::ensure!(
            (50..=60_000).contains(&self.refiner.timeout_ms),
            "refiner.timeout_ms must be between 50 and 60000"
        );
        anyhow::ensure!(
            (1..=8_192).contains(&self.refiner.max_output_tokens),
            "refiner.max_output_tokens must be between 1 and 8192"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips() {
        let encoded = toml::to_string(&VoiceConfig::default()).unwrap();
        let decoded: VoiceConfig = toml::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.audio.sample_rate, 16_000);
        assert_eq!(decoded.hotkey, "control+shift+space");
        assert_eq!(decoded.asr.backend, AsrBackend::Whisper);
        assert_eq!(decoded.refiner.backend, RefinerBackend::Identity);
        assert_eq!(decoded.insertion.mode, InsertionMode::Auto);
    }

    #[test]
    fn invalid_chunk_size_is_rejected() {
        let mut config = VoiceConfig::default();
        config.audio.chunk_ms = 10;
        assert!(config.validate().is_err());
    }
}
