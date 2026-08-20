use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PERMISSION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    Microphone,
    Accessibility,
    InputMonitoring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatusSnapshot {
    pub schema_version: u32,
    pub subject_executable: PathBuf,
    pub microphone: PermissionState,
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PermissionRequirementError {
    #[error("microphone permission is not active")]
    Microphone,
    #[error("Accessibility permission is not active")]
    Accessibility,
}

pub fn validate_runtime_permissions(
    snapshot: &PermissionStatusSnapshot,
    _hotkey: &str,
    requires_insertion: bool,
) -> Result<(), PermissionRequirementError> {
    if snapshot.microphone != PermissionState::Authorized {
        return Err(PermissionRequirementError::Microphone);
    }
    if requires_insertion && snapshot.accessibility != PermissionState::Authorized {
        return Err(PermissionRequirementError::Accessibility);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorized() -> PermissionStatusSnapshot {
        PermissionStatusSnapshot {
            schema_version: PERMISSION_SCHEMA_VERSION,
            subject_executable: PathBuf::from("/tmp/voice-input"),
            microphone: PermissionState::Authorized,
            accessibility: PermissionState::Authorized,
            input_monitoring: PermissionState::Authorized,
        }
    }

    #[test]
    fn default_hotkey_does_not_require_input_monitoring() {
        let mut snapshot = authorized();
        snapshot.input_monitoring = PermissionState::Denied;

        assert!(validate_runtime_permissions(&snapshot, "control+shift+space", true).is_ok());
    }

    #[test]
    fn function_hotkey_uses_accessibility_for_event_listening() {
        let mut snapshot = authorized();
        snapshot.input_monitoring = PermissionState::Denied;

        assert!(validate_runtime_permissions(&snapshot, "fn", true).is_ok());
    }

    #[test]
    fn stdout_mode_does_not_require_accessibility() {
        let mut snapshot = authorized();
        snapshot.accessibility = PermissionState::Denied;

        assert!(validate_runtime_permissions(&snapshot, "control+shift+space", false).is_ok());
    }

    #[test]
    fn microphone_is_always_required() {
        let mut snapshot = authorized();
        snapshot.microphone = PermissionState::Denied;

        let error =
            validate_runtime_permissions(&snapshot, "control+shift+space", false).unwrap_err();

        assert_eq!(error, PermissionRequirementError::Microphone);
    }

    #[test]
    fn text_insertion_requires_accessibility() {
        let mut snapshot = authorized();
        snapshot.accessibility = PermissionState::Denied;

        let error =
            validate_runtime_permissions(&snapshot, "control+shift+space", true).unwrap_err();

        assert_eq!(error, PermissionRequirementError::Accessibility);
    }
}
