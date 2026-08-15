use crate::domain::LatencyReport;
use crate::ports::LatencySink;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySample {
    pub recorded_at_ms: u64,
    pub report: LatencyReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub samples: usize,
    pub first_audio_samples: usize,
    pub first_audio_ms: Option<Percentiles>,
    pub stop_to_inserted_ms: Percentiles,
    pub total_ms: Percentiles,
    pub finalize_ms: Percentiles,
    pub refine_ms: Percentiles,
    pub insert_ms: Percentiles,
    pub total_audio_chunks: u64,
    pub total_dropped_chunks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Percentiles {
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub max: u64,
}

impl LatencySummary {
    pub fn from_samples(samples: &[LatencySample]) -> anyhow::Result<Self> {
        anyhow::ensure!(!samples.is_empty(), "latency history contains no samples");
        let reports: Vec<_> = samples.iter().map(|sample| &sample.report).collect();
        let first_audio: Vec<_> = reports
            .iter()
            .filter_map(|report| report.first_audio_ms)
            .collect();
        Ok(Self {
            samples: reports.len(),
            first_audio_samples: first_audio.len(),
            first_audio_ms: (!first_audio.is_empty()).then(|| percentiles(first_audio.into_iter())),
            stop_to_inserted_ms: percentiles(
                reports.iter().map(|report| report.stop_to_inserted_ms),
            ),
            total_ms: percentiles(reports.iter().map(|report| report.total_ms)),
            finalize_ms: percentiles(reports.iter().map(|report| report.finalize_ms)),
            refine_ms: percentiles(reports.iter().map(|report| report.refine_ms)),
            insert_ms: percentiles(reports.iter().map(|report| report.insert_ms)),
            total_audio_chunks: reports.iter().map(|report| report.audio_chunks).sum(),
            total_dropped_chunks: reports.iter().map(|report| report.dropped_chunks).sum(),
        })
    }
}

pub struct FileLatencySink {
    sender: Option<mpsc::SyncSender<LatencySample>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FileLatencySink {
    pub fn new(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let (sender, receiver) = mpsc::sync_channel(256);
        let worker = thread::Builder::new()
            .name("voice-input-latency".to_owned())
            .spawn(move || latency_writer(file, path, receiver))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }
}

impl LatencySink for FileLatencySink {
    fn record(&mut self, report: &LatencyReport) -> anyhow::Result<()> {
        let sample = LatencySample {
            recorded_at_ms: unix_time_ms(),
            report: report.clone(),
        };
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("latency writer is closed"))?
            .try_send(sample)
            .map_err(|error| anyhow::anyhow!("latency history queue rejected a sample: {error}"))
    }
}

impl Drop for FileLatencySink {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("latency history worker panicked");
            }
        }
    }
}

pub fn read_latency_history(path: &Path) -> anyhow::Result<Vec<LatencySample>> {
    let file = std::fs::File::open(path)?;
    let mut samples = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let sample = serde_json::from_str(&line).map_err(|error| {
            anyhow::anyhow!(
                "invalid latency history at {} line {}: {error}",
                path.display(),
                index + 1
            )
        })?;
        samples.push(sample);
    }
    Ok(samples)
}

fn latency_writer(mut file: File, path: PathBuf, receiver: mpsc::Receiver<LatencySample>) {
    while let Ok(sample) = receiver.recv() {
        match serde_json::to_writer(&mut file, &sample) {
            Ok(()) => {
                if let Err(error) = file.write_all(b"\n") {
                    tracing::warn!(path = %path.display(), %error, "failed to terminate latency sample");
                    continue;
                }
                if let Err(error) = file.flush() {
                    tracing::warn!(path = %path.display(), %error, "failed to flush latency history");
                }
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "failed to serialize latency sample");
            }
        }
    }
}

fn percentiles(values: impl Iterator<Item = u64>) -> Percentiles {
    let mut values: Vec<_> = values.collect();
    values.sort_unstable();
    Percentiles {
        min: values[0],
        p50: nearest_rank(&values, 50),
        p95: nearest_rank(&values, 95),
        max: values[values.len() - 1],
    }
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let rank = (percentile * sorted.len()).div_ceil(100);
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
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

    fn sample(value: u64) -> LatencySample {
        LatencySample {
            recorded_at_ms: value,
            report: LatencyReport {
                capture_ms: value,
                first_audio_ms: Some(1),
                finalize_ms: value,
                refine_ms: value,
                insert_ms: value,
                stop_to_inserted_ms: value,
                total_ms: value,
                audio_chunks: 2,
                audio_samples: 2_560,
                dropped_chunks: u64::from(value == 100),
            },
        }
    }

    #[test]
    fn summary_uses_nearest_rank_percentiles() {
        let samples: Vec<_> = (1..=100).map(sample).collect();
        let summary = LatencySummary::from_samples(&samples).unwrap();

        assert_eq!(summary.stop_to_inserted_ms.p50, 50);
        assert_eq!(summary.stop_to_inserted_ms.p95, 95);
        assert_eq!(summary.first_audio_samples, 100);
        assert_eq!(summary.first_audio_ms.unwrap().p95, 1);
        assert_eq!(summary.total_audio_chunks, 200);
        assert_eq!(summary.total_dropped_chunks, 1);
    }

    #[test]
    fn file_sink_round_trips_jsonl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("latency.jsonl");
        let mut sink = FileLatencySink::new(&path).unwrap();
        sink.record(&sample(42).report).unwrap();
        drop(sink);

        let samples = read_latency_history(&path).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].report.total_ms, 42);
    }

    #[test]
    fn summary_marks_first_audio_unavailable_when_no_chunk_arrives() {
        let mut only = sample(1);
        only.report.first_audio_ms = None;

        let summary = LatencySummary::from_samples(&[only]).unwrap();

        assert_eq!(summary.first_audio_samples, 0);
        assert!(summary.first_audio_ms.is_none());
    }
}
