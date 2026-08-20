use crate::{PermissionKind, PermissionState, PermissionStatusSnapshot, PERMISSION_SCHEMA_VERSION};
use anyhow::Context;
use block2::RcBlock;
use macos_accessibility_client::accessibility::{
    application_is_trusted, application_is_trusted_with_prompt,
};
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use std::sync::mpsc;
use std::time::Duration;

pub fn snapshot() -> anyhow::Result<PermissionStatusSnapshot> {
    Ok(PermissionStatusSnapshot {
        schema_version: PERMISSION_SCHEMA_VERSION,
        subject_executable: std::env::current_exe()?,
        microphone: microphone_state()?,
        accessibility: boolean_state(application_is_trusted()),
        input_monitoring: boolean_state(unsafe { CGPreflightListenEventAccess() }),
    })
}

pub fn request(permission: PermissionKind) -> anyhow::Result<PermissionStatusSnapshot> {
    match permission {
        PermissionKind::Microphone => request_microphone()?,
        PermissionKind::Accessibility => {
            let _ = application_is_trusted_with_prompt();
        }
        PermissionKind::InputMonitoring => unsafe {
            let _ = CGRequestListenEventAccess();
        },
    }
    snapshot()
}

fn microphone_state() -> anyhow::Result<PermissionState> {
    let media_type = unsafe { AVMediaTypeAudio }
        .ok_or_else(|| anyhow::anyhow!("AVFoundation did not expose the audio media type"))?;
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    Ok(match status {
        AVAuthorizationStatus::NotDetermined => PermissionState::NotDetermined,
        AVAuthorizationStatus::Restricted => PermissionState::Restricted,
        AVAuthorizationStatus::Denied => PermissionState::Denied,
        AVAuthorizationStatus::Authorized => PermissionState::Authorized,
        _ => PermissionState::Unknown,
    })
}

fn request_microphone() -> anyhow::Result<()> {
    if microphone_state()? != PermissionState::NotDetermined {
        return Ok(());
    }
    let media_type = unsafe { AVMediaTypeAudio }
        .ok_or_else(|| anyhow::anyhow!("AVFoundation did not expose the audio media type"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |granted: Bool| {
        let _ = sender.send(granted.as_bool());
    });
    unsafe {
        AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &handler);
    }
    receiver
        .recv_timeout(Duration::from_secs(120))
        .map(|_| ())
        .context("timed out waiting for the microphone permission response")
}

fn boolean_state(authorized: bool) -> PermissionState {
    if authorized {
        PermissionState::Authorized
    } else {
        PermissionState::NotDetermined
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}
