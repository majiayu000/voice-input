use fs2::FileExt;
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelPreset {
    Fast,
    Balanced,
    Quality,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelArtifact {
    pub preset: &'static str,
    pub file_name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub description: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InstalledModel {
    pub artifact: ModelArtifact,
    pub path: PathBuf,
    pub reused_existing: bool,
}

impl ModelPreset {
    pub fn artifact(self) -> ModelArtifact {
        match self {
            Self::Fast => ModelArtifact {
                preset: "fast",
                file_name: "ggml-tiny-q5_1.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
                sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
                size_bytes: 32_152_673,
                description: "Small smoke-test model; lowest latency and lowest accuracy.",
            },
            Self::Balanced => ModelArtifact {
                preset: "balanced",
                file_name: "ggml-small-q5_1.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
                sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
                size_bytes: 190_085_487,
                description: "Recommended starting point for interactive multilingual dictation.",
            },
            Self::Quality => ModelArtifact {
                preset: "quality",
                file_name: "ggml-large-v3-turbo-q5_0.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
                sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
                size_bytes: 574_041_195,
                description: "Recommended quality preset for Apple Silicon with more memory.",
            },
        }
    }
}

pub fn model_catalog() -> [ModelArtifact; 3] {
    [
        ModelPreset::Fast.artifact(),
        ModelPreset::Balanced.artifact(),
        ModelPreset::Quality.artifact(),
    ]
}

pub async fn install_model(
    preset: ModelPreset,
    model_directory: &Path,
    force: bool,
) -> anyhow::Result<InstalledModel> {
    let artifact = preset.artifact();
    tokio::fs::create_dir_all(model_directory).await?;
    let install_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(model_directory.join(".install.lock"))?;
    install_lock.try_lock_exclusive().map_err(|error| {
        anyhow::anyhow!("another model installation is already running: {error}")
    })?;
    let destination = model_directory.join(artifact.file_name);
    let temporary = model_directory.join(format!(".{}.partial", artifact.file_name));
    if destination.exists() {
        let existing_hash = sha256_file(&destination).await?;
        if existing_hash == artifact.sha256 {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Ok(InstalledModel {
                artifact,
                path: destination,
                reused_existing: true,
            });
        }
        anyhow::ensure!(
            force,
            "model {} exists but its SHA-256 is {}; pass --force to replace it",
            destination.display(),
            existing_hash
        );
    }

    download_with_resume(&artifact, &temporary).await?;
    let digest = sha256_file(&temporary).await?;
    if digest != artifact.sha256 {
        let _ = tokio::fs::remove_file(&temporary).await;
        anyhow::bail!(
            "model SHA-256 mismatch: expected {}, received {}",
            artifact.sha256,
            digest
        );
    }
    tokio::fs::rename(&temporary, &destination).await?;
    Ok(InstalledModel {
        artifact,
        path: destination,
        reused_existing: false,
    })
}

async fn download_with_resume(artifact: &ModelArtifact, temporary: &Path) -> anyhow::Result<()> {
    const ATTEMPTS: usize = 4;
    crate::tls::install_default_provider();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut last_error = None;

    for attempt in 1..=ATTEMPTS {
        let existing = tokio::fs::metadata(temporary)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        anyhow::ensure!(
            existing <= artifact.size_bytes,
            "partial model exceeds the declared artifact size"
        );
        if existing == artifact.size_bytes {
            return Ok(());
        }

        let mut request = client.get(artifact.url);
        if existing > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
        }
        let response = match request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(error.to_string());
                retry_delay(attempt).await;
                continue;
            }
        };
        let resumed = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let start = if resumed { existing } else { 0 };
        if let Some(length) = response.content_length() {
            anyhow::ensure!(
                length == artifact.size_bytes - start,
                "model download size changed: expected {} remaining bytes, server reports {}",
                artifact.size_bytes - start,
                length
            );
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true);
        if resumed {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let mut file = options.open(temporary).await?;
        let mut downloaded = start;
        let mut stream = response.bytes_stream();
        let mut stream_error = None;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => {
                    downloaded = downloaded.saturating_add(chunk.len() as u64);
                    anyhow::ensure!(
                        downloaded <= artifact.size_bytes,
                        "model download exceeded its declared size"
                    );
                    file.write_all(&chunk).await?;
                }
                Err(error) => {
                    stream_error = Some(error.to_string());
                    break;
                }
            }
        }
        file.sync_all().await?;
        drop(file);
        if downloaded == artifact.size_bytes && stream_error.is_none() {
            return Ok(());
        }
        last_error = stream_error.or_else(|| {
            Some(format!(
                "download stopped at {downloaded} of {} bytes",
                artifact.size_bytes
            ))
        });
        tracing::warn!(
            attempt,
            downloaded,
            expected = artifact.size_bytes,
            "model download interrupted; retrying with HTTP Range"
        );
        retry_delay(attempt).await;
    }

    anyhow::bail!(
        "model download failed after {ATTEMPTS} attempts: {}",
        last_error.unwrap_or_else(|| "unknown download failure".to_owned())
    )
}

async fn retry_delay(attempt: usize) {
    if attempt < 4 {
        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
    }
}

async fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut HashWriter(&mut hasher))?;
        Ok::<_, anyhow::Error>(format!("{:x}", hasher.finalize()))
    })
    .await?
}

struct HashWriter<'a>(&'a mut Sha256);

impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_unique_names_and_valid_hashes() {
        let catalog = model_catalog();
        for (index, artifact) in catalog.iter().enumerate() {
            assert_eq!(artifact.sha256.len(), 64);
            assert!(artifact
                .sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit()));
            assert!(artifact.size_bytes > 1_000_000);
            assert!(catalog[index + 1..]
                .iter()
                .all(|other| other.file_name != artifact.file_name));
        }
    }
}
