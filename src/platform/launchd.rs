use crate::service::{ServiceError, ServiceManager, ServicePaths, ServiceStatus, SERVICE_LABEL};
use crate::status::RuntimeSnapshot;
use std::fs::{OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub struct LaunchdServiceManager {
    paths: ServicePaths,
    domain: String,
}

impl LaunchdServiceManager {
    pub fn new(paths: ServicePaths) -> Self {
        // SAFETY: getuid has no preconditions and does not mutate process state.
        let uid = unsafe { libc::getuid() };
        Self {
            paths,
            domain: format!("gui/{uid}"),
        }
    }

    fn target(&self) -> String {
        format!("{}/{SERVICE_LABEL}", self.domain)
    }

    fn launchd_details(&self) -> Result<LaunchdDetails, ServiceError> {
        let output = Command::new("/bin/launchctl")
            .args(["print", &self.target()])
            .output()?;
        if !output.status.success() {
            return Ok(LaunchdDetails::default());
        }
        Ok(parse_launchctl_print(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn run_launchctl(
        &self,
        action: &'static str,
        arguments: &[&str],
    ) -> Result<Output, ServiceError> {
        let output = Command::new("/bin/launchctl").args(arguments).output()?;
        if output.status.success() {
            return Ok(output);
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(ServiceError::Launchctl {
            action,
            detail: if detail.is_empty() {
                format!("exit status {}", output.status)
            } else {
                detail
            },
        })
    }

    fn read_runtime(&self) -> Result<Option<RuntimeSnapshot>, ServiceError> {
        match std::fs::read(&self.paths.runtime_status) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn wait_until_unloaded(&self) -> Result<(), ServiceError> {
        // CoreAudio/TCC can keep a helper in kernel IPC briefly after launchd
        // accepts bootout. Keep the CLI synchronous without waiting forever.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.launchd_details()?.loaded {
            if Instant::now() >= deadline {
                return Err(ServiceError::Launchctl {
                    action: "bootout",
                    detail: "service remained loaded after bootout".to_owned(),
                });
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
}

impl ServiceManager for LaunchdServiceManager {
    fn install(&self, executable: &Path) -> Result<ServiceStatus, ServiceError> {
        for log in [&self.paths.stdout_log, &self.paths.stderr_log] {
            if let Some(parent) = log.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        write_atomically(
            &self.paths.app_bundle.join("Contents/Info.plist"),
            render_app_info_plist().as_bytes(),
            0o644,
        )?;
        copy_executable_atomically(executable, &self.paths.binary)?;
        sign_app_bundle(&self.paths.app_bundle)?;
        remove_if_exists(&self.paths.data_dir.join("bin/voice-input"))?;
        write_atomically(
            &self.paths.launch_agent,
            render_plist(&self.paths).as_bytes(),
            0o644,
        )?;
        self.status()
    }

    fn start(&self) -> Result<ServiceStatus, ServiceError> {
        if !self.paths.launch_agent.exists() || !self.paths.binary.exists() {
            return Err(ServiceError::NotInstalled);
        }
        if self.launchd_details()?.loaded {
            self.run_launchctl("kickstart", &["kickstart", "-k", &self.target()])?;
        } else {
            self.run_launchctl(
                "bootstrap",
                &[
                    "bootstrap",
                    &self.domain,
                    self.paths.launch_agent.to_string_lossy().as_ref(),
                ],
            )?;
            self.run_launchctl("kickstart", &["kickstart", "-k", &self.target()])?;
        }
        self.status()
    }

    fn stop(&self) -> Result<ServiceStatus, ServiceError> {
        if self.launchd_details()?.loaded {
            self.run_launchctl("bootout", &["bootout", &self.target()])?;
            self.wait_until_unloaded()?;
        }
        self.status()
    }

    fn status(&self) -> Result<ServiceStatus, ServiceError> {
        let details = self.launchd_details()?;
        let runtime = self
            .read_runtime()?
            .filter(|snapshot| details.loaded && details.pid == Some(snapshot.pid));
        Ok(ServiceStatus {
            label: SERVICE_LABEL,
            installed: self.paths.launch_agent.exists() && self.paths.binary.exists(),
            loaded: details.loaded,
            launchd_state: details.state,
            pid: details.pid,
            last_exit_code: details.last_exit_code,
            runtime,
            paths: self.paths.clone(),
        })
    }

    fn uninstall(&self) -> Result<ServiceStatus, ServiceError> {
        if self.launchd_details()?.loaded {
            self.run_launchctl("bootout", &["bootout", &self.target()])?;
            self.wait_until_unloaded()?;
        }
        remove_if_exists(&self.paths.launch_agent)?;
        remove_directory_if_exists(&self.paths.app_bundle)?;
        remove_if_exists(&self.paths.runtime_status)?;
        self.status()
    }
}

fn render_app_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Voice Input</string>
  <key>CFBundleExecutable</key>
  <string>voice-input</string>
  <key>CFBundleIdentifier</key>
  <string>com.lifcc.voiceinput</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Voice Input</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Voice Input needs microphone access for local speech recognition.</string>
</dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION")
    )
}

fn sign_app_bundle(app_bundle: &Path) -> Result<(), ServiceError> {
    let identity =
        std::env::var("VOICE_INPUT_CODESIGN_IDENTITY").unwrap_or_else(|_| "-".to_owned());
    let output = Command::new("/usr/bin/codesign")
        .args([
            "--force",
            "--sign",
            &identity,
            "--options",
            "runtime",
            "--timestamp=none",
        ])
        .arg(app_bundle)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(ServiceError::Packaging(if detail.is_empty() {
        format!("codesign exited with {}", output.status)
    } else {
        detail
    }))
}

fn remove_directory_if_exists(path: &Path) -> Result<(), ServiceError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LaunchdDetails {
    loaded: bool,
    state: Option<String>,
    pid: Option<u32>,
    last_exit_code: Option<i32>,
}

fn parse_launchctl_print(output: &str) -> LaunchdDetails {
    let mut details = LaunchdDetails {
        loaded: true,
        ..LaunchdDetails::default()
    };
    for line in output.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("state = ") {
            details.state = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("pid = ") {
            details.pid = value.trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("last exit code = ") {
            details.last_exit_code = value.trim().parse().ok();
        }
    }
    details
}

fn render_plist(paths: &ServicePaths) -> String {
    let binary = xml_escape(&paths.binary.to_string_lossy());
    let config = xml_escape(&paths.config.to_string_lossy());
    let stdout_log = xml_escape(&paths.stdout_log.to_string_lossy());
    let stderr_log = xml_escape(&paths.stderr_log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{binary}</string>
    <string>--config</string>
    <string>{config}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <false/>
  <key>KeepAlive</key>
  <false/>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>ThrottleInterval</key>
  <integer>1</integer>
  <key>StandardOutPath</key>
  <string>{stdout_log}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_log}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>RUST_LOG</key>
    <string>voice_input=info</string>
  </dict>
</dict>
</plist>
"#
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn copy_executable_atomically(source: &Path, destination: &Path) -> Result<(), ServiceError> {
    if source == destination {
        return Ok(());
    }
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "installed binary path has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".voice-input.{}.install", std::process::id()));
    let result = (|| -> Result<(), ServiceError> {
        std::fs::copy(source, &temporary)?;
        std::fs::set_permissions(&temporary, Permissions::from_mode(0o755))?;
        std::fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn write_atomically(path: &Path, bytes: &[u8], mode: u32) -> Result<(), ServiceError> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "service file path has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".voice-input.{}.tmp", std::process::id()));
    let result = (|| -> Result<(), ServiceError> {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::set_permissions(&temporary, Permissions::from_mode(mode))?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn remove_if_exists(path: &Path) -> Result<(), ServiceError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_uses_stable_paths_and_escapes_xml() {
        let directory = tempfile::tempdir().unwrap();
        let mut paths = ServicePaths::in_root(directory.path());
        paths.config = directory.path().join("config/a&b.toml");

        let plist = render_plist(&paths);

        assert!(plist.contains("com.lifcc.voiceinput"));
        assert!(plist.contains("a&amp;b.toml"));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <false/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n  <false/>"));
    }

    #[test]
    fn launchctl_status_parser_extracts_operational_fields() {
        let details = parse_launchctl_print(
            r#"
                gui/501/com.lifcc.voiceinput = {
                    state = running
                    pid = 42001
                    last exit code = 0
                }
            "#,
        );

        assert_eq!(
            details,
            LaunchdDetails {
                loaded: true,
                state: Some("running".to_owned()),
                pid: Some(42001),
                last_exit_code: Some(0),
            }
        );
    }

    #[test]
    fn installer_copies_binary_and_prepares_log_directory() {
        let directory = tempfile::tempdir().unwrap();
        let paths = ServicePaths::in_root(directory.path());
        let source = directory.path().join("source-binary");
        std::fs::write(&source, b"voice-input-test").unwrap();
        let manager = LaunchdServiceManager {
            paths: paths.clone(),
            domain: "gui/0".to_owned(),
        };

        let status = manager.install(&source).unwrap();

        assert!(status.installed);
        assert_eq!(std::fs::read(&paths.binary).unwrap(), b"voice-input-test");
        assert!(paths.stdout_log.parent().unwrap().is_dir());
        assert!(paths.launch_agent.is_file());
        let app_info =
            std::fs::read_to_string(paths.app_bundle.join("Contents/Info.plist")).unwrap();
        assert!(app_info.contains("<string>com.lifcc.voiceinput</string>"));
        assert!(app_info.contains("<key>NSMicrophoneUsageDescription</key>"));
        let mode = std::fs::metadata(&paths.binary)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755);
    }
}
