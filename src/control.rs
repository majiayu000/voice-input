use crate::config::{InsertionMode, RefinerBackend, RefinerFailureMode, VoiceConfig};
use crate::models::model_catalog;
use crate::service::{ServicePaths, ServiceStatus};
use crate::status::RuntimeSnapshot;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONTROL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct ControlSnapshot {
    pub schema_version: u32,
    pub generated_at_ms: u64,
    pub service: ServiceStatus,
    pub settings: ControlSettings,
    pub models: Vec<ControlModel>,
    pub recent_runtime: Option<RuntimeSnapshot>,
}

#[derive(Debug, Serialize)]
pub struct ControlSettings {
    pub config_path: PathBuf,
    pub hotkey: String,
    pub audible_feedback: bool,
    pub language: String,
    pub insertion_mode: String,
    pub active_model_path: Option<PathBuf>,
    pub refiner_enabled: bool,
    pub refiner_base_url: String,
    pub refiner_model: String,
    pub refiner_api_key_env: Option<String>,
    pub refiner_allow_remote: bool,
    pub refiner_failure_mode: String,
    pub refiner_system_prompt: String,
    pub vad_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ControlModel {
    pub preset: &'static str,
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub description: &'static str,
    pub path: PathBuf,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SettingsPatch {
    pub hotkey: Option<String>,
    pub audible_feedback: Option<bool>,
    pub language: Option<String>,
    pub insertion_mode: Option<crate::config::InsertionMode>,
    pub refiner_enabled: Option<bool>,
    pub refiner_base_url: Option<String>,
    pub refiner_model: Option<String>,
    pub refiner_api_key_env: Option<String>,
    pub refiner_allow_remote: Option<bool>,
    pub refiner_failure_mode: Option<crate::config::RefinerFailureMode>,
    pub refiner_system_prompt: Option<String>,
    pub vad_enabled: Option<bool>,
}

pub fn build_control_snapshot(
    config: &VoiceConfig,
    config_path: &Path,
    paths: &ServicePaths,
    service: ServiceStatus,
) -> ControlSnapshot {
    let active_model = config.asr.model_path.as_deref();
    let models = model_catalog()
        .into_iter()
        .map(|artifact| {
            let path = paths.data_dir.join("models").join(artifact.file_name);
            ControlModel {
                preset: artifact.preset,
                file_name: artifact.file_name,
                size_bytes: artifact.size_bytes,
                description: artifact.description,
                installed: path.is_file(),
                active: active_model == Some(path.as_path()),
                path,
            }
        })
        .collect();
    let recent_runtime = crate::status::read_runtime_snapshot(&paths.runtime_status)
        .ok()
        .flatten();

    ControlSnapshot {
        schema_version: CONTROL_SCHEMA_VERSION,
        generated_at_ms: unix_time_ms(),
        service,
        settings: ControlSettings {
            config_path: config_path.to_path_buf(),
            hotkey: config.hotkey.clone(),
            audible_feedback: config.feedback.audible,
            language: config.asr.language.clone(),
            insertion_mode: insertion_mode_name(config.insertion.mode).to_owned(),
            active_model_path: config.asr.model_path.clone(),
            refiner_enabled: config.refiner.backend == RefinerBackend::OpenaiCompatible,
            refiner_base_url: config.refiner.base_url.clone(),
            refiner_model: config.refiner.model.clone(),
            refiner_api_key_env: config.refiner.api_key_env.clone(),
            refiner_allow_remote: config.refiner.allow_remote,
            refiner_failure_mode: failure_mode_name(config.refiner.failure_mode).to_owned(),
            refiner_system_prompt: config.refiner.system_prompt.clone(),
            vad_enabled: config.asr.vad.enabled,
        },
        models,
        recent_runtime,
    }
}

pub fn apply_settings_patch(
    config_path: Option<&Path>,
    patch: SettingsPatch,
) -> anyhow::Result<(VoiceConfig, PathBuf)> {
    let mut config = VoiceConfig::load(config_path)?;
    if let Some(hotkey) = patch.hotkey {
        config.hotkey = hotkey;
    }
    if let Some(audible) = patch.audible_feedback {
        config.feedback.audible = audible;
    }
    if let Some(language) = patch.language {
        config.asr.language = language;
    }
    if let Some(mode) = patch.insertion_mode {
        config.insertion.mode = mode;
    }
    if let Some(enabled) = patch.refiner_enabled {
        config.refiner.backend = if enabled {
            RefinerBackend::OpenaiCompatible
        } else {
            RefinerBackend::Identity
        };
    }
    if let Some(base_url) = patch.refiner_base_url {
        config.refiner.base_url = base_url;
    }
    if let Some(model) = patch.refiner_model {
        config.refiner.model = model;
    }
    if let Some(api_key_env) = patch.refiner_api_key_env {
        config.refiner.api_key_env = (!api_key_env.trim().is_empty()).then_some(api_key_env);
    }
    if let Some(allow_remote) = patch.refiner_allow_remote {
        config.refiner.allow_remote = allow_remote;
    }
    if let Some(failure_mode) = patch.refiner_failure_mode {
        config.refiner.failure_mode = failure_mode;
    }
    if let Some(system_prompt) = patch.refiner_system_prompt {
        config.refiner.system_prompt = system_prompt;
    }
    if let Some(enabled) = patch.vad_enabled {
        config.asr.vad.enabled = enabled;
    }
    config.validate()?;
    let path = config.save(config_path)?;
    Ok((config, path))
}

fn insertion_mode_name(value: InsertionMode) -> &'static str {
    match value {
        InsertionMode::Auto => "auto",
        InsertionMode::Accessibility => "accessibility",
        InsertionMode::Clipboard => "clipboard",
    }
}

fn failure_mode_name(value: RefinerFailureMode) -> &'static str {
    match value {
        RefinerFailureMode::Bypass => "bypass",
        RefinerFailureMode::Fail => "fail",
    }
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
    fn settings_patch_updates_only_named_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        VoiceConfig::default().save(Some(&path)).unwrap();
        let patch = SettingsPatch {
            hotkey: Some("fn".to_owned()),
            audible_feedback: Some(false),
            language: Some("zh".to_owned()),
            refiner_api_key_env: Some(String::new()),
            ..SettingsPatch::default()
        };

        let (updated, saved) = apply_settings_patch(Some(&path), patch).unwrap();

        assert_eq!(saved, path);
        assert_eq!(updated.hotkey, "fn");
        assert!(!updated.feedback.audible);
        assert_eq!(updated.asr.language, "zh");
        assert!(updated.refiner.api_key_env.is_none());
        assert_eq!(updated.audio.sample_rate, 16_000);
    }
}
